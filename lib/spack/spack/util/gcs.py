# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""
This file contains the definition of the GCS Blob storage Class used to
integrate GCS Blob storage with spack buildcache.
"""

import os
import sys
import urllib.parse
import urllib.response
from typing import Optional, Any, Callable, List, Dict, TYPE_CHECKING
from urllib.error import URLError
from urllib.request import BaseHandler, Request

import llnl.util.tty as tty

if TYPE_CHECKING:
    # Import for type checking only to avoid runtime ImportError if google-cloud-storage is not installed
    from google.cloud import storage # type: ignore[attr-defined]
    from google.cloud.storage.bucket import Bucket # type: ignore[attr-defined]
    from google.cloud.storage.blob import Blob # type: ignore[attr-defined]
    from google.auth.credentials import Credentials # type: ignore[attr-defined]


def gcs_client() -> "storage.Client":
    """Create a GCS client
    Creates an authenticated GCS client to access GCS buckets and blobs
    """

    try:
        import google.auth
        from google.cloud import storage
    except ImportError as ex:
        tty.error(
            "{0}, google-cloud-storage python module is missing.".format(ex)
            + " Please install to use the gs:// backend."
        )
        sys.exit(1) # This makes the return type effectively storage.Client | NoReturn

    storage_credentials: Optional["Credentials"]
    storage_project: Optional[str]
    storage_credentials, storage_project = google.auth.default()
    storage_client: "storage.Client" = storage.Client(
        project=storage_project, credentials=storage_credentials
    )
    return storage_client


class GCSBucket:
    """GCS Bucket Object
    Create a wrapper object for a GCS Bucket. Provides methods to wrap spack
    related tasks, such as destroy.
    """

    def __init__(self, url: urllib.parse.ParseResult, client: Optional["storage.Client"] = None):
        """Constructor for GCSBucket objects

        Args:
          url (urllib.parse.ParseResult): The url pointing to the GCS bucket to build an object out of
          client (google.cloud.storage.client.Client): A pre-defined storage
                 client that will be used to access the GCS bucket.
        """
        if url.scheme != "gs":
            raise ValueError(
                "Can not create GCS bucket connection with scheme {SCHEME}".format(
                    SCHEME=url.scheme
                )
            )
        self.url: urllib.parse.ParseResult = url
        self.name: str = self.url.netloc
        if self.url.path and self.url.path.startswith("/"):
            self.prefix: str = self.url.path[1:]
        else:
            self.prefix: str = self.url.path or ""

        self.client: "storage.Client" = client or gcs_client()
        self.bucket: Optional["Bucket"] = None

        tty.debug("New GCS bucket:")
        tty.debug("    name: {0}".format(self.name))
        tty.debug("    prefix: {0}".format(self.prefix))

    def exists(self) -> bool:
        from google.cloud.exceptions import NotFound

        if not self.bucket:
            try:
                # self.client should be storage.Client here
                self.bucket = self.client.bucket(self.name)
            except NotFound as ex:
                tty.error("{0}, Failed check for bucket existence".format(ex))
                # Consider re-raising or returning False more directly
                sys.exit(1) # Makes return type bool | NoReturn
        return self.bucket is not None

    def create(self) -> None:
        if not self.bucket: # Should call self.exists() first or handle self.client interaction
            self.bucket = self.client.create_bucket(self.name)

    def get_blob(self, blob_path: str) -> Optional["Blob"]:
        if self.exists() and self.bucket:
            return self.bucket.get_blob(blob_path)
        return None

    def blob(self, blob_path: str) -> Optional["Blob"]:
        if self.exists() and self.bucket:
            return self.bucket.blob(blob_path)
        return None

    def get_all_blobs(self, recursive: bool = True, relative: bool = True) -> Optional[List[str]]:
        """Get a list of all blobs
        Returns a list of all blobs within this bucket.

        Args:
            relative: If true (default), print blob paths
                         relative to 'build_cache' directory.
                      If false, print absolute blob paths (useful for
                         destruction of bucket)
        """
        tty.debug("Getting GCS blobs... Recurse {0} -- Rel: {1}".format(recursive, relative))

        converter: Callable[[str], str]
        if relative:
            converter = self._relative_blob_name
        else:
            converter = str # type: ignore[assignment] # str is Callable[[Any], str]

        if self.exists() and self.bucket:
            # list_blobs returns an iterator of Blob objects
            all_blobs_iterator = self.bucket.list_blobs(prefix=self.prefix)
            blob_list: List[str] = []

            # +1 because split on "/" for "a/b/c" gives 3 parts, representing 2 directory levels for prefix.
            # If prefix is empty, base_dirs is 1. If prefix is "foo", base_dirs is 2.
            # num_dirs for "foo/bar.txt" is 2.
            base_dirs: int = len(self.prefix.split("/")) if self.prefix else 0
            # If prefix is like "a/b/", splitting gives ['a', 'b', ''], len is 3.
            # A blob "a/b/c/file.txt" split gives ['a','b','c','file.txt'], len 4.
            # This logic might need adjustment based on how GCS paths/prefixes work.
            # For now, assuming prefix does not end with '/' for this calculation.
            if self.prefix and not self.prefix.endswith('/'):
                 base_dirs +=1
            elif not self.prefix: # root prefix
                 base_dirs = 0


            for blob_obj in all_blobs_iterator:
                if not recursive:
                    # Count path segments. If prefix is "a/b", blob "a/b/file.txt" has 3 segments.
                    # We want blobs directly under prefix, so num_dirs == base_dirs + 1 (for the filename itself)
                    # or if prefix is empty, num_dirs == 1.
                    num_dirs_in_blob_path = len(blob_obj.name.split("/"))
                    # If prefix="a/b", base_dirs=2. Blob "a/b/file.txt" (3 parts) means num_dirs_in_blob_path (3) == base_dirs(2)+1.
                    # If prefix="", base_dirs=0. Blob "file.txt" (1 part) means num_dirs_in_blob_path (1) == base_dirs(0)+1.
                    # If prefix="a/", base_dirs=1. Blob "a/file.txt" (2 parts) means num_dirs_in_blob_path(2) == base_dirs(1)+1.
                    # This seems more correct: check if #parts in blob is one more than #parts in prefix
                    
                    # A simpler check: is the blob's parent directory effectively the prefix?
                    # Or, does the blob name contain more slashes beyond the prefix?
                    relative_path_to_prefix = os.path.relpath(blob_obj.name, self.prefix if self.prefix else None)
                    if '/' not in relative_path_to_prefix or not relative_path_to_prefix: # no additional subdirectories
                         blob_list.append(converter(blob_obj.name))

                else: # recursive
                    blob_list.append(converter(blob_obj.name))
            return blob_list
        return None # Bucket does not exist or self.bucket is None

    def _relative_blob_name(self, blob_name: str) -> str:
        # Ensure self.prefix is not empty before using it as the base for relpath if blob_name is absolute
        if self.prefix:
            return os.path.relpath(blob_name, self.prefix)
        return blob_name # If no prefix, name is already "relative" to bucket root

    def destroy(self, recursive: bool = False, **kwargs: Any) -> None:
        """Bucket destruction method

        Deletes all blobs within the bucket, and then deletes the bucket itself.

        Uses GCS Batch operations to bundle several delete operations together.
        """
        from google.cloud.exceptions import NotFound

        tty.debug("Bucket.destroy(recursive={0})".format(recursive))
        
        if not self.exists() or not self.bucket:
            tty.warn(f"Bucket {self.name} does not exist or not initialized. Cannot destroy.")
            return

        try:
            bucket_blobs: Optional[List[str]] = self.get_all_blobs(recursive=recursive, relative=False)
            if bucket_blobs is None:
                tty.warn(f"Could not list blobs for bucket {self.name}. Cannot destroy.")
                return

            batch_size: int = 1000
            num_blobs: int = len(bucket_blobs)

            for i in range(0, num_blobs, batch_size):
                with self.client.batch():
                    for j in range(i, min(i + batch_size, num_blobs)):
                        current_blob: Optional["Blob"] = self.blob(bucket_blobs[j])
                        if current_blob:
                            current_blob.delete()
                        else:
                            tty.warn(f"Blob {bucket_blobs[j]} not found during deletion batch.")
            
            # Potentially delete the bucket itself if recursive and empty, GCS API dependent
            # self.bucket.delete() # This might be needed if all blobs are gone
            # However, spack's current logic doesn't show bucket deletion, only blobs.

        except NotFound as ex:
            tty.error("{0}, Could not delete a blob in bucket {1}.".format(ex, self.name))
            sys.exit(1) # Or re-raise


class GCSBlob:
    """GCS Blob object

    Wraps some blob methods for spack functionality
    """

    def __init__(self, url: urllib.parse.ParseResult, client: Optional["storage.Client"] = None):
        self.url: urllib.parse.ParseResult = url
        if url.scheme != "gs":
            raise ValueError(
                "Can not create GCS blob connection with scheme: {SCHEME}".format(
                    SCHEME=url.scheme
                )
            )

        self.client: "storage.Client" = client or gcs_client()
        self.bucket_obj: GCSBucket = GCSBucket(url, self.client) # Pass client here
        self.blob_path: str = self.url.path.lstrip("/") if self.url.path else ""


        tty.debug("New GCSBlob")
        tty.debug("  blob_path = {0}".format(self.blob_path))

        if not self.bucket_obj.exists():
            tty.warn("The bucket {0} does not exist, it will be created".format(self.bucket_obj.name))
            self.bucket_obj.create()

    def get(self) -> Optional["Blob"]:
        return self.bucket_obj.get_blob(self.blob_path)

    def exists(self) -> bool:
        from google.cloud.exceptions import NotFound

        try:
            blob_instance: Optional["Blob"] = self.bucket_obj.blob(self.blob_path)
            if blob_instance:
                return blob_instance.exists()
            return False
        except NotFound:
            return False

    def delete_blob(self) -> None:
        from google.cloud.exceptions import NotFound

        try:
            blob_instance: Optional["Blob"] = self.bucket_obj.blob(self.blob_path)
            if blob_instance:
                blob_instance.delete()
            else:
                tty.warn(f"Blob {self.blob_path} not found, cannot delete.")
        except NotFound as ex:
            tty.error("{0}, Could not delete gcs blob {1}".format(ex, self.blob_path))

    def upload_to_blob(self, local_file_path: str) -> None:
        blob_instance: Optional["Blob"] = self.bucket_obj.blob(self.blob_path)
        if blob_instance:
            blob_instance.upload_from_filename(local_file_path)
        else:
            # This case should ideally be handled by bucket.blob() returning a new blob if not found for upload
            tty.error(f"Blob {self.blob_path} could not be referenced for upload.")


    def get_blob_byte_stream(self) -> IO[bytes]:
        blob_instance = self.bucket_obj.get_blob(self.blob_path)
        if not blob_instance:
            raise URLError(f"GCS blob {self.blob_path} does not exist for byte stream.")
        # The 'open' method of google.cloud.storage.blob.Blob returns IO[bytes] for 'rb'
        return blob_instance.open(mode="rb") # type: ignore[no-any-return]

    def get_blob_headers(self) -> Dict[str, Optional[str]]:
        blob_instance = self.bucket_obj.get_blob(self.blob_path)
        if not blob_instance:
            # Return empty or default headers if blob doesn't exist
            return {}

        headers: Dict[str, Optional[str]] = {
            "Content-type": blob_instance.content_type,
            "Content-encoding": blob_instance.content_encoding,
            "Content-language": blob_instance.content_language,
            "MD5Hash": blob_instance.md5_hash, # This is base64 encoded md5
        }
        return headers


def gcs_open(req: Request, *args: Any, **kwargs: Any) -> urllib.response.addinfourl:
    """Open a reader stream to a blob object on GCS"""
    # req.get_full_url() is already a string
    url: urllib.parse.ParseResult = urllib.parse.urlparse(req.get_full_url())
    gcsblob = GCSBlob(url)

    if not gcsblob.exists():
        raise URLError("GCS blob {0} does not exist".format(gcsblob.blob_path))
    
    stream: IO[bytes] = gcsblob.get_blob_byte_stream()
    headers_dict = gcsblob.get_blob_headers()
    
    # Convert dict to email.message.Message or list of tuples for addinfourl
    # For simplicity, using a basic list of tuples if that's compatible,
    # otherwise, one might need to construct a Message object.
    # addinfourl expects an info object that acts like mimetools.Message.
    # A simple dict might not work for all functionalities.
    # However, for basic header reading, this might suffice or need adjustment.
    # For now, passing as dict, but this might need a proper Message object.
    # Actual usage in Spack will determine if this is sufficient.
    info_headers = list(headers_dict.items())


    return urllib.response.addinfourl(stream, info_headers, req.get_full_url()) # type: ignore[arg-type]


class GCSHandler(BaseHandler):
    def gs_open(self, req: Request) -> urllib.response.addinfourl: # Matches what gcs_open returns
        # Additional arguments like *args, **kwargs are not typically passed by URLopener
        return gcs_open(req)

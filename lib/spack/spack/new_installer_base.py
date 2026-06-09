# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Abstract base classes for the new_installer TUI terminal state, stdin reading, and IPC.

Kept in a leaf module (no imports from new_installer.py or the platform modules) so that
new_installer_posix and new_installer_windows can import from here without introducing a
circular dependency."""

import abc
import codecs
import io
import os
import re
import selectors
import socket
import sys
import threading
import warnings
from multiprocessing import Process
from multiprocessing.connection import Connection
from typing import TYPE_CHECKING, Callable, Dict, Optional, Union

import spack.database
import spack.spec
import spack.util.lock

if TYPE_CHECKING:
    from spack.new_installer import BuildStatus


class StdinReaderBase:
    """Base class for platform-specific non-blocking stdin reading with UTF-8 decoding.

    The input is the backing file descriptor for stdin (instead of the TextIOWrapper) to
    avoid double buffering issues: the event loop triggers when the fd is ready to read, and if we
    do a partial read from the TextIOWrapper, it will likely drain the fd and buffer the remainder
    internally, which the event loop is not aware of, and user input doesn't come through."""

    def __init__(self) -> None:
        #: Handle multi-byte UTF-8 characters
        self.decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        #: For stripping out arrow and navigation keys
        self.ansi_escape_re = re.compile(r"\x1b\[[0-9;]*[A-Za-z~]")

    def _decode(self, raw: bytes) -> str:
        return self.ansi_escape_re.sub("", self.decoder.decode(raw))

    def read(self) -> str:
        raise NotImplementedError


class BaseTerminalState(abc.ABC):
    """Abstract base for platform-specific terminal state management."""

    def __init__(
        self,
        selector: selectors.BaseSelector,
        build_status: "BuildStatus",
        on_suspend: Optional[Callable[[], None]] = None,
        on_resume: Optional[Callable[[], None]] = None,
    ) -> None:
        self.selector = selector
        self.build_status = build_status
        self.on_suspend = on_suspend
        self.on_resume = on_resume

    @classmethod
    def stdout_is_interactive(cls) -> bool:
        return sys.stdout.isatty()

    @classmethod
    def stdin_is_interactive(cls) -> bool:
        return sys.stdin.isatty()

    @abc.abstractmethod
    def create_stdin_reader(self) -> StdinReaderBase:
        pass

    @abc.abstractmethod
    def setup(self) -> None:
        pass

    @abc.abstractmethod
    def teardown_input(self) -> None:
        """Restore input settings and signal handlers. Called before the final UI render."""
        pass

    @abc.abstractmethod
    def teardown_output(self) -> None:
        """Restore output settings. Called after the final UI render."""
        pass

    def teardown(self) -> None:
        self.teardown_input()
        self.teardown_output()

    @abc.abstractmethod
    def enter_foreground(self) -> None:
        pass

    @abc.abstractmethod
    def enter_background(self) -> None:
        pass

    @abc.abstractmethod
    def handle_continue(self) -> None:
        pass

    @abc.abstractmethod
    def drain_sigwinch(self) -> None:
        """Drain the platform-specific sigwinch notification channel."""
        pass

    @abc.abstractmethod
    def should_enter_foreground(self) -> bool:
        """Return True if the process should switch from headless to foreground mode."""
        pass


#: Channel type for IPC between build and UI processes (Connection on POSIX, socket on Windows).
StateChannel = Union[Connection, socket.socket]

#: Size of the output buffer for child processes
OUTPUT_BUFFER_SIZE = 32768


class DatabaseAction:
    """Base class for objects that need to be persisted to the database."""

    __slots__ = ("spec", "prefix_lock")

    spec: "spack.spec.Spec"
    prefix_lock: Optional[spack.util.lock.Lock]

    def save_to_db(self, db: spack.database.Database) -> None: ...

    def release_prefix_lock(self) -> None:
        if self.prefix_lock is not None:
            try:
                self.prefix_lock.release_write()
            except Exception:
                pass
        self.prefix_lock = None


class FdInfo:
    """Information about a file descriptor mapping."""

    __slots__ = ("pid", "name")

    def __init__(self, pid: int, name: str) -> None:
        self.pid = pid
        self.name = name


class ChildInfo(DatabaseAction, abc.ABC):
    """Abstract base for per-process information about a running build."""

    __slots__ = ("proc", "output_r_conn", "state_r_conn", "control_w_conn", "log_path", "explicit")

    def __init__(
        self,
        proc: Process,
        spec: spack.spec.Spec,
        output_r_conn: StateChannel,
        state_r_conn: StateChannel,
        control_w_conn: StateChannel,
        log_path: str,
        explicit: bool = False,
    ) -> None:
        self.proc = proc
        self.spec = spec
        self.output_r_conn = output_r_conn
        self.state_r_conn = state_r_conn
        self.control_w_conn = control_w_conn
        self.log_path = log_path
        self.explicit = explicit
        self.prefix_lock: Optional[spack.util.lock.Lock] = None

    def save_to_db(self, db: spack.database.Database) -> None:
        return db._add(self.spec, explicit=self.explicit)

    def _join_and_return(self) -> int:
        """Close the control channel, join the child process, and return its exit code."""
        self.control_w_conn.close()
        self.proc.join()
        exit_code = self.proc.exitcode
        assert exit_code is not None, "Finished build should have exit code set"
        if hasattr(self.proc, "close"):
            self.proc.close()
        return exit_code

    @abc.abstractmethod
    def register_with_selector(self, selector: selectors.BaseSelector, pid: int) -> None:
        pass

    @abc.abstractmethod
    def close(self, selector: selectors.BaseSelector) -> int:
        pass


class Tee(abc.ABC):
    """Abstract base for intercepting and teeing process output to a log file and parent."""

    def __init__(self, control: StateChannel, parent: StateChannel, log_path: str) -> None:
        self.control = control
        self.parent = parent
        self.log_path = log_path
        # sys.stdout and sys.stderr may have been replaced with file objects under pytest, so
        # redirect their file descriptors in addition to the original fds 1 and 2.
        fds = {sys.stdout.fileno(), sys.stderr.fileno(), 1, 2}
        self.saved_fds: Dict[int, int] = {fd: os.dup(fd) for fd in fds}
        log_file = open(log_path, "ab")
        r, w = os.pipe()
        self.tee_thread = threading.Thread(target=self.run, args=(r, log_file), daemon=True)
        self.tee_thread.start()
        for fd in fds:
            os.dup2(w, fd)
        self._setup_handles()
        os.close(w)

    @abc.abstractmethod
    def run(self, log_r: int, log_file: "io.BufferedWriter") -> None:
        pass

    def _setup_handles(self) -> None:
        """Hook called after dup2; override on Windows to redirect Win32 stdout/stderr handles."""

    def _restore_handles(self) -> None:
        """Hook called after fd restoration; override on Windows to restore Win32 handles."""

    def close(self) -> None:
        # Flush and restore stdout/stderr before joining: restoring closes the last reference to
        # the write end of the pipe, which unblocks the tee thread. We also flush first because
        # between sys.exit and the actual process exit buffers may be flushed, and can cause exit
        # code 120 (witnessed under pytest+coverage on macOS).
        sys.stdout.flush()
        sys.stderr.flush()
        for fd, saved_fd in self.saved_fds.items():
            os.dup2(saved_fd, fd)
            os.close(saved_fd)
        self.tee_thread.join()
        self.control.close()
        self.parent.close()
        self._restore_handles()


class JobServer(abc.ABC):
    """Abstract base for POSIX and Windows jobservers."""

    def __init__(self, num_jobs: int) -> None:
        #: Keep track of how many tokens Spack itself has acquired, which is used to release them.
        self.tokens_acquired = 0
        #: The number of jobs to run concurrently. This translates to `num_jobs - 1` tokens in the
        #: jobserver.
        self.num_jobs = num_jobs
        #: The target number of jobs to run concurrently, which may differ from num_jobs if the
        #: user has requested a decrease in parallelism, but we haven't consumed enough tokens to
        #: reflect that yet. This value is used in the UI. The invariant is that self.target_jobs
        #: can only be modified if self.created is True.
        self.target_jobs = num_jobs
        self.fifo_path: Optional[str] = None
        self.created = False
        self.r: int = -1
        self.w: int = -1
        self.r_conn: Optional[Connection] = None
        self.w_conn: Optional[Connection] = None
        self._init_channels()

    @abc.abstractmethod
    def _init_channels(self) -> None:
        pass

    @abc.abstractmethod
    def makeflags(self, gmake: Optional[spack.spec.Spec]) -> str:
        pass

    def update_selector(self, selector: selectors.BaseSelector, wake_on_jobserver: bool) -> None:
        """Register or unregister the jobserver read fd with the selector. No-op on Windows."""

    def has_target_parallelism(self) -> bool:
        return self.num_jobs == self.target_jobs

    def increase_parallelism(self) -> None:
        """Add one token to the jobserver to increase parallelism."""
        if not self.created:
            return
        self.target_jobs += 1
        # If a decrease was pending, don't add a token.
        if self.target_jobs <= self.num_jobs:
            return
        os.write(self.w, b"+")
        self.num_jobs += 1

    def decrease_parallelism(self) -> None:
        """Request an eventual concurrency decrease by 1."""
        if not self.created or self.target_jobs <= 1:
            return
        self.target_jobs -= 1
        self.maybe_discard_tokens()

    def maybe_discard_tokens(self) -> None:
        """Try to reduce parallelism by discarding tokens. No-op on Windows."""

    def acquire(self, jobs: int) -> int:
        """Try to acquire up to ``jobs`` tokens. Returns the number acquired. 0 on Windows."""
        return 0

    def release(self) -> None:
        """Release a token back to the jobserver. No-op on Windows."""

    def _close_channels(self) -> None:
        """Close platform channels and clean up resources. No-op on Windows."""

    def close(self) -> None:
        if self.created and self.num_jobs > 1:
            if self.tokens_acquired != 0:
                warnings.warn("Spack failed to release jobserver tokens", stacklevel=2)
            else:
                total = self.num_jobs - 1
                drained = self.acquire(total)
                if drained != total:
                    n = total - drained
                    warnings.warn(
                        f"{n} jobserver {'token was' if n == 1 else 'tokens were'} not released "
                        "by the build processes. This can indicate that the build ran with "
                        "limited parallelism.",
                        stacklevel=2,
                    )
        self._close_channels()

# Ideas to Try

Ordered roughly by expected impact (highest first).

## spack.cmd.__init__ — top-level imports to lazify

- Move `import spack.concretize` to function scope (saves ~78ms cumulative import tree:
  concretize → compilers.config → detection → detection.common → …)
- Move `import spack.environment as ev` to function scope (~21ms)
- Move `import spack.store` to function scope (unknown, but spack.database is heavy)
- Move `import spack.repo` to function scope (~19ms) — check if used at module level
- Move `import spack.spec` to function scope (~30ms self) — heavily used, may be hard
- Move `import spack.spec_parser` to function scope (~3ms self)
- Move `import spack.util.spack_yaml as syaml` to function scope (~7ms)
- Move `import spack.user_environment as uenv` to function scope
- Move `from spack.llnl.util.filesystem import join_path` — check if used at module level

## spack.main — top-level imports to lazify

- Move `import spack.solver.asp` to function scope (~8ms)
- Move `import spack.environment` / `import spack.environment as ev` to function scope
- Move `import spack.store` to function scope
- Move `import spack.platforms` to function scope
- Move `import spack.util.debug` to function scope
- Move `import spack.util.environment` to function scope

## Deeper cuts

- `spack.detection` (~34ms) is pulled in via `spack.compilers.config`; check if
  `compilers.config` needs it at import time or only when detecting compilers
- `spack.vendor.jsonschema` (~28ms): loaded via config schema validation — investigate if
  schema validation can be deferred until config is actually read
- `spack.fetch_strategy` (~15ms): pulled in somewhere — trace which module imports it
  eagerly and whether that import can be deferred
- `spack.oci.opener` (~11ms): very unlikely to be needed for `spack list` — trace the
  import chain and defer
- `spack.util.remote_file_cache` (~11ms): trace and defer
- `urllib.request` (~14ms): imported transitively; check if HTTP-related code can be
  deferred until a network operation is actually requested

## Longer-term / structural

- `spack list` loads the full package repo at startup; investigate if repo listing can
  stream package names without loading all package metadata
- Consider a fast path in `spack.main` that detects simple read-only commands (list, find
  with no install) and skips environment/store setup entirely

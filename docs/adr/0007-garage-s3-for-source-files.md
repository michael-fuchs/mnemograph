# Source files live in Garage, a self-hosted S3 store

**Status:** superseded (2026-10-31): the document service indexes files where they are and keeps no copies, so no object store is needed.

The original files of sources (uploads, fetched documents) are stored in Garage, a self-hosted S3-compatible object store running as its own container on a single node, rather than on mnemograph's local disk or as BLOBs in SQLite. Objects are content-addressed by SHA-256; metadata and extracted text stay in SQLite, which remains the source of truth for everything else. Garage gives future services (ADR 0004) the same storage and keeps the database small. mnemograph accesses it through the `object_store` crate, whose local filesystem and in-memory backends serve tests.

## Considered Options

- **A content-addressed directory next to the database:** no extra container, but no shared storage for other services.
- **BLOBs in SQLite:** a single file and simple consistency, but a large database and slow `VACUUM INTO` backups.
- **Hosted object storage:** no service to run, but monthly cost against the budget and files off the server.
- **MinIO:** its community edition stopped shipping binaries in 2025.

## Consequences

One more container, credentials and a backup path. Single-node Garage has no redundancy, so backups (metadata snapshots plus the data directory) are the only protection; the metadata uses the SQLite engine with fsync on. SQLite and Garage are not updated atomically: an object can exist without a source referring to it, which is harmless because objects are content-addressed and can be cleaned up.

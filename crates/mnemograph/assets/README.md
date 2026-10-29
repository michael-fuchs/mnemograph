# Web UI assets

Everything the web UI loads is served from this directory, which is compiled into the binary. There is no JavaScript build step. Third-party libraries are vendored as published, prebuilt files. To update one, download the new release, check it against the source below, replace the file, and update this table, the file name in `src/web/mod.rs` and the licence file.

| File | Library | Source | SHA-256 of the uncompressed file | Licence |
|---|---|---|---|---|
| `datastar-1.0.4.js` | Datastar 1.0.4 | GitHub release `v1.0.4`, `bundles/datastar.js` (git blob `360caf359e27bf577275dae6a93816946ad1f1b7`) | `727844adfc825ee651fb93c544a2a739986f9a21820a94524b35f0cac470cf91` | MIT, `datastar-LICENSE.md` |
| `mermaid-12.2.0.min.js.gz` | Mermaid 12.2.0, gzipped | npm `mermaid@12.2.0`, `dist/mermaid.min.js` | `a8a6a6f805b074a8261a2825c86f99c8054acb3b35f50bb7e24a4f9f2c3c3b9f` | MIT, `mermaid-LICENSE.txt` |
| `cytoscape-3.34.4.min.js.gz` | Cytoscape.js 3.34.4, gzipped | npm `cytoscape@3.34.4` (tarball integrity `sha512-msY99tMY8iWTg+sIPGfSqDcPNOt4xcj+v/K/7qUFaCUxxlqnMkUYLBjRUzn9W5CU/R2hrmtJa5+UtovrEM2ctg==`), `dist/cytoscape.min.js` | `081591ab735b2b6a3b31a3c60f8f3a3e2fee8c92939e0c7820ad1208571b6cd8` | MIT, `cytoscape-LICENSE.txt` |

Our own files are `style.css` and `graph.js`. `graph.js` defines the `<mnemo-graph>` element, which draws a neighborhood graph with Cytoscape.

# Providers, conversion and storage for v2 (research, 2026-10-28/29)

These are the facts gathered for the v2 architecture (`docs/plans/mnemograph-v2.md`). They are facts, not recommendations.

Versions and dates come from crates.io, GitHub, PyPI, Hugging Face or the vendors' docs as of the research date. **UNVERIFIED** marks a claim that has only a secondary source or is an estimate.

## 1. Local CPU embeddings in Rust

### Crates

- **`fastembed` 7.5.0 (2026-10-23, Apache-2.0)**
  - Uses ONNX Runtime through `ort`. The default build downloads the runtime binaries; `ort-load-dynamic` is the alternative.
  - Models download from Hugging Face on first use. Offline use goes through `try_new_from_user_defined`.
  - Multilingual models include multilingual-e5-small/base/large, paraphrase-multilingual-mpnet, bge-m3, embeddinggemma-300m and Qwen3-Embedding (Qwen3 runs on candle).
  - Source: https://github.com/Anush008/fastembed-rs
- **`ort` 2.0.0-rc.15 (2026-10-19)**: still a release candidate.
- **`candle-core` 0.11.0**: pure Rust.
- **`model2vec-rs` 0.3.0 (2026-09-19, MIT)**
  - Static embeddings with no neural runtime.
  - It can embed the model in the binary.

### Models

| Model | Dim | Max tokens | Size | Licence | Note |
|---|---|---|---|---|---|
| multilingual-e5-small | 384 | 512 | 118 MB qint8 ONNX | MIT | `query:`/`passage:` prefixes |
| Qwen3-Embedding-0.6B | 32–1024 | 32k | 1.19 GB | Apache-2.0 | MMTEB multilingual 64.33 |
| EmbeddingGemma-300m | 768 | 2048 | 1.23 GB | Gemma terms, gated | MMTEB multilingual 61.15 |
| bge-m3 | 1024 | 8192 | 2.27 GB ONNX | MIT | MMTEB multilingual 59.56 |
| potion-multilingual-128M (model2vec) | 256 | unlimited | 512 MB | MIT | MTEB 47.31 |
| jina-embeddings v3/v5 | 1024 | 8192 | — | **CC-BY-NC-4.0** | non-commercial |

- RAM is roughly the model size plus activations (**UNVERIFIED**).
- No reliable CPU throughput figure exists for 2 vCPU.

## 2. Hosted embeddings

| Provider | Price per 1M tokens | Notes |
|---|---|---|
| Mistral `mistral-embed` | $0.10 (official pricing page, 2026-10-29) | Batch API −50 %. EU hosting by default via `api.eu.mistral.ai` (**UNVERIFIED**). |
| Voyage | voyage-4 $0.06, voyage-4-lite $0.02 | https://docs.voyageai.com/docs/pricing |
| OpenAI | text-embedding-3-small $0.02, -large $0.13 | |

## 3. Vector search at mnemograph's scale

- **Memory:** 50k vectors take 77 MB at 384 dims, 154 MB at 768 dims and 205 MB at 1024 dims (f32). int8 needs a quarter of that.
- **Speed:** exact search is estimated at tens of ms per query (**UNVERIFIED**, no direct benchmark found).
- **Crates:** `simsimd` 6.5 (SIMD distances), `usearch` 2.26 (HNSW), `hnsw_rs` 0.3.
- **Hybrid with Tantivy:** run both searches separately, then fuse with reciprocal rank fusion, score = Σ 1/(k + rank), usually k = 60. Crates: `rank-fusion`, `rrf`.

## 4. Converting to Markdown

### Rust crates

- **`pdf_oxide` 0.3.78 (MIT/Apache)**: PDF to text, Markdown or HTML, with tables; no OCR. https://github.com/yfedoseev/pdf_oxide
- **`pdf-extract` 0.12, `lopdf` 0.46**: plain text and low-level PDF access.
- **`mupdf` 0.8**: **AGPL**.
- **`htmd` 0.5.6 (Apache-2.0)**: HTML to Markdown. `html2md` is GPL.
- **`dom_smoothie` 0.19.0 (MIT)**
  - Readability port.
  - Outputs Markdown.
  - Extracts metadata: title, byline, excerpt, published time, JSON-LD.
- **`webpage` 2.1.0 (MIT)**: title, description, language, OpenGraph, schema.org.
- **`docx-rs`**: mainly a writer; reading is limited (**UNVERIFIED**).

### External tools

- **pandoc 3.13**: GPL-2.0, no PDF input.
- **markitdown 0.1.9**: MIT, Python.
- **docling 2.144**: MIT, but the CPU image is 4.4 GB.
- **marker 2.0**: model weights under a modified OpenRAIL-M licence.
- **MinerU**: about 2 GB RAM even in its basic tier.
- **PyMuPDF**: AGPL.

### Mistral OCR

- **Endpoint:** `POST /v1/ocr`.
- **Models:** OCR 4.1 and OCR 3 are current.
- **Input:** PDF, PPTX or DOCX by URL or base64; images.
- **Output:** Markdown per page, tables as Markdown or HTML, images, header and footer.
- **Price:** OCR 4.1 $4 per 1,000 pages, OCR 3 $2 per 1,000 pages; Batch API −50 %.
- **Limits:** 50 MB and 1,000 pages (**UNVERIFIED**).
- Sources: https://docs.mistral.ai/api/endpoint/ocr, https://docs.mistral.ai/inference/pricing

## 5. LLM access from Rust

- **No official Rust SDK exists** from Anthropic or Mistral.
- **`genai` 0.7.2 (2026-10-28, MIT/Apache-2.0)**
  - Mistral goes through the OpenAI adapter, with tool calling, streaming with tool calls and usage reporting.
  - Custom reqwest client and endpoints.
  - Embedding API (`embed`, `embed_batch`). Mistral embeddings are confirmed only by reading the source (**UNVERIFIED**, to test).
  - Ollama is supported for chat and embeddings.
  - Releases are frequent; 0.7.0 had breaking changes.
  - https://github.com/jeremychone/rust-genai
- **`rig-core` 0.45.0 (2026-10-19, MIT)**
  - Mistral embeddings are documented (`mistral::embedding::MISTRAL_EMBED`); Anthropic and Ollama are supported.
  - A heavier framework.
  - "Releases contain breaking changes" in every minor version.
- **Anthropic MCP connector** (beta): the Messages API can call a remote MCP server directly, but the server must be publicly reachable, and the application has to obtain the OAuth token itself.

### Mistral prices (2026-10-29, per 1M tokens, input/output)

| Model | Input | Output |
|---|---|---|
| Small 4 | $0.15 | $0.60 |
| Large 4 | $0.70 | $2.10 |
| Medium 3.5 | $1.50 | $7.50 |

The data processing addendum allows training on customer data unless the customer opts out.

## 6. Object storage

### Garage 2.5.0 (Docker image `dxflrs/garage:v2.5.0`, AGPL-3.0, used unmodified over S3)

- **Single node:** `replication_factor = 1` (documented as meant for tests), so backups are the only redundancy.
- **Metadata engine:** SQLite. It is more robust than LMDB after an unclean shutdown.
- **Durability settings:** `metadata_fsync` and `data_fsync` are recommended for a single node.
- **S3 support:** multipart uploads, presigned URLs, path-style addressing. No versioning, object lock or ACLs. Conditional PUT is undocumented (**UNVERIFIED**).
- **Backup:**
  - `metadata_auto_snapshot_interval` keeps the last 2 snapshots, or run `garage meta snapshot` by hand.
  - Restore: stop Garage, replace the database with the snapshot, then run `garage repair -a --yes tables`.
  - Data blocks are content-addressed and immutable.
- Sources: https://garagehq.deuxfleurs.fr/documentation/reference-manual/configuration/, `.../s3-compatibility/`, `.../operations/recovering/`

### Other stores and clients

- **MinIO:** the community edition stopped shipping binaries in 2025.
- **`object_store` 0.14.2 (MIT/Apache-2.0)**
  - The S3 backend uses path-style addressing by default; set `with_endpoint`, `with_allow_http` and a region that matches Garage.
  - Presigned URLs via `Signer`; multipart uploads.
  - `LocalFileSystem` and `InMemory` backends for tests.
  - Rely on conditional put only after testing it against Garage.
- **Alternatives:** `aws-sdk-s3` 1.156 (heaviest), `opendal` 0.59, `rust-s3` 0.38.

# Obtaining & launching llama-server

Everything about getting the binary onto a machine and starting it the way you
want. For the meaning of individual flags see `cli-reference.md`.

## Obtaining the binary

| Method | How |
|--------|-----|
| **Package manager** | `brew install llama.cpp` (macOS/Linux Homebrew), `winget install llama.cpp` (Windows), Nix `nixpkgs#llama-cpp`, Arch `pacman -S llama.cpp`. |
| **Prebuilt release** | Download the OS/arch asset from the project's GitHub Releases; `llama-server` is inside. Pick a CPU build or a matching GPU build (cuda/vulkan/etc.). |
| **Build from source** | See below — the only way to target a specific GPU backend or a bleeding-edge commit. |
| **Docker** | GHCR images (see below) — no local toolchain needed. |

### Build from source (CMake)

```bash
git clone https://github.com/ggml-org/llama.cpp
cd llama.cpp
cmake -B build                         # CPU-only
# GPU backends (pick one):
#   -DGGML_CUDA=ON      NVIDIA (needs CUDA Toolkit)
#   -DGGML_METAL=ON     Apple Silicon (default on macOS)
#   -DGGML_VULKAN=ON    cross-vendor GPUs
#   -DGGML_HIP=ON       AMD ROCm
cmake --build build --config Release -j
./build/bin/llama-server --help
```

Building with TLS support additionally needs OpenSSL (`-DLLAMA_OPENSSL=ON`).

### Docker (GHCR)

Images are published at `ghcr.io/ggml-org/llama.cpp`:

- Tag families: `:server` (server only), `:full` (all tools), `:light`.
- GPU variants are suffixed: `-cuda`, `-rocm`, `-vulkan`, `-intel`, `-musa`.

```bash
# CPU:
docker run -p 8080:8080 -v "$PWD/models:/models" \
  ghcr.io/ggml-org/llama.cpp:server \
  -m /models/model.gguf -c 4096 --host 0.0.0.0

# NVIDIA GPU (note --gpus all AND -ngl):
docker run --gpus all -p 8080:8080 -v "$PWD/models:/models" \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/model.gguf -c 4096 -ngl 99 --host 0.0.0.0
```

Inside a container you **must** bind `--host 0.0.0.0`, or the published port will
refuse connections (the default `127.0.0.1` is container-local).

## Loading a model

- **Local file:** `-m ./model.gguf`
- **Hugging Face auto-download:** `-hf <user>/<repo>[:quant]` — e.g.
  `-hf ggml-org/gemma-3-1b-it-GGUF`. Without a `:quant` suffix it picks a default
  (commonly `Q4_K_M`). Use `-hff <file>` to pin a specific GGUF file in the repo.
  Set `HF_TOKEN` for gated/private repos.
- **Direct URL:** `-mu <url>` downloads a GGUF from any URL.

## Common launch recipes

```bash
# CPU-only, modest context:
llama-server -m model.gguf -c 4096

# Full GPU offload:
llama-server -m model.gguf -c 8192 -ngl 99

# Multi-GPU, split by layer across 2 GPUs:
llama-server -m model.gguf -ngl 99 -sm layer -ts 0.5,0.5

# Serve to the network, 4 concurrent slots, behind an API key:
llama-server -m model.gguf -c 16384 -np 4 --host 0.0.0.0 --port 8080 \
  --api-key "$MY_KEY"
```

## Networking

- `--host HOST` — default `127.0.0.1` (localhost only). Use `0.0.0.0` for
  external access / Docker.
- `--port PORT` — default `8080`.
- `--api-prefix PREFIX` — serve the API under a path prefix (e.g. behind a proxy).
- `-to, --timeout N` — server read/write timeout in seconds (default `3600`).

## Authentication & TLS

- `--api-key KEY` — require `Authorization: Bearer KEY`. Multiple keys can be
  comma-separated.
- `--api-key-file FILE` — one key per line.
- `--ssl-key-file FILE` + `--ssl-cert-file FILE` — serve HTTPS directly (the
  binary must be built with OpenSSL).

> Note: when no `--api-key` is set, the server accepts any request and ignores
> the `Authorization` header entirely.

## Web UI

The browser UI is **enabled by default** at the server root (`http://host:port`).

- Disable it with `--no-webui`.
- In current builds the flag is `--ui` / `--no-ui`; the older `--webui` spelling
  was renamed (see llama.cpp issue #18155) — check `--help` on your build.

## Configuration via environment variables

Almost every flag has an `LLAMA_ARG_*` environment variable. **Command-line
arguments override environment variables.** Useful for containers / IaC:

| Env var | Equivalent flag |
|---------|-----------------|
| `LLAMA_ARG_MODEL` | `-m` |
| `LLAMA_ARG_CTX_SIZE` | `-c` |
| `LLAMA_ARG_N_GPU_LAYERS` | `-ngl` |
| `LLAMA_ARG_N_PARALLEL` | `-np` |
| `LLAMA_ARG_HOST` | `--host` |
| `LLAMA_ARG_PORT` | `--port` |
| `LLAMA_ARG_N_PREDICT` | `-n` |
| `LLAMA_API_KEY` | `--api-key` |
| `HF_TOKEN` | Hugging Face auth for `-hf` |

Boolean flags accept `1`/`true`/`on`/`enabled` (and the inverse to disable). To
turn off a default-on flag via env, use its `LLAMA_ARG_NO_*` form (e.g.
`LLAMA_ARG_NO_MMAP=1`).

`--offline` (or `LLAMA_ARG_OFFLINE=1`) forces use of the local cache and blocks
all network access — fail fast instead of hanging on a download.

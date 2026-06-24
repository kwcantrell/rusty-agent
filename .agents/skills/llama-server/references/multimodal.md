# Multimodal (vision & audio)

`llama-server` serves multimodal models (image and/or audio input) through
llama.cpp's `mtmd` stack. You need two things: a multimodal **model** and its
matching **projector** (`mmproj`) file.

## Enabling multimodal

```bash
# Local model + projector:
llama-server -m vision-model.gguf --mmproj mmproj-model.gguf -c 8192 -ngl 99

# Hugging Face: the projector is usually auto-downloaded with the model:
llama-server -hf ggml-org/gemma-3-4b-it-GGUF -c 8192 -ngl 99
```

Flags:
- `-mm, --mmproj FILE` — path to the multimodal projector.
- `-mmu, --mmproj-url URL` — download the projector from a URL.
- `--mmproj-auto` / `--no-mmproj-auto` — auto-use a projector shipped with the
  model (default on). Pass `--no-mmproj-auto` to force text-only.
- `--mmproj-offload` — offload the projector to GPU (default on).
- `--image-min-tokens N` / `--image-max-tokens N` — bound tokens spent per image
  (dynamic-resolution models).
- `--media-path DIR` — directory the server may read local media files from.

> The projector **must** match the model. A mismatched or missing `mmproj`
> produces garbage or an error — see `troubleshooting.md`.

## Sending an image (OpenAI shape)

Use the `content` array with an `image_url`. A base64 data URI works without any
file server:

```bash
B64=$(base64 -w0 cat.png)
curl http://localhost:8080/v1/chat/completions -H 'Content-Type: application/json' -d "{
  \"messages\":[{\"role\":\"user\",\"content\":[
    {\"type\":\"text\",\"text\":\"What is in this image?\"},
    {\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,$B64\"}}
  ]}]
}"
```

A plain `http(s)://` URL also works in `image_url.url` when the server can reach
it.

## Sending audio

Audio-capable models accept `input_audio` content parts:

```json
{"role":"user","content":[
  {"type":"text","text":"Transcribe this."},
  {"type":"input_audio","input_audio":{"data":"<base64>","format":"wav"}}
]}
```

## Notes

- **Native `/completion`:** when using media there, the number of media markers
  in the prompt must match the number of items in the data array.
- Vision/audio inflates prompt token counts (each image is many tokens) — size
  `-c` accordingly (see `performance-tuning.md`).
- Confirm modality support via `GET /props` (it reports the model's modalities).

<p align="center">
  <img src="public/icon-192.png" width="72" alt="">
</p>

<h1 align="center">tally-ho</h1>

Photograph a receipt, a local vision model pulls the line items out of it, then
upload your card statement and reconcile it charge by charge. Each person in Settings
has a description of what they tend to buy, and that's what decides who pays what.
A final CSV can be exported with the approved reconciliations.

Leptos and axum, SQLite through toasty, extraction via Ollama.

## Development

```
nix develop          # or let direnv do it
cargo leptos watch
```

Needs an Ollama instance with both models pulled: one to read a receipt photo,
one to turn the text into line items.

| Variable | Default |
|---|---|
| `OLLAMA_URL` | `http://localhost:11434` |
| `OLLAMA_MODEL` | `gemma4:12b` |
| `OLLAMA_OCR_MODEL` | `glm-ocr:q8_0` — empty to let `OLLAMA_MODEL` read the photo itself |
| `OLLAMA_OCR_CONTEXT` | `8192` — `num_ctx` for the OCR model; too small silently truncates the transcript |
| `OLLAMA_ASSIGN_MODEL` | same as `OLLAMA_MODEL` — the one that guesses whose each item is |
| `OLLAMA_KEEP_ALIVE` | `-1m` — how long Ollama holds the models in VRAM; negative never unloads, empty leaves the server's default |
| `MAX_IMAGE_EDGE` | `1600` — longest edge a photo is downscaled to before it's read |
| `CURRENCY` | `USD` — the ISO code your statements are in |
| `DATABASE_URL` | `sqlite:./data/tally-ho.db` |
| `DATA_DIR` | `./data` |

## Container

```
nix build .#image
nix run .#image.copyToDockerDaemon
docker run -p 3000:3000 -v tally-ho:/data \
  -e OLLAMA_URL=http://your-host:11434 tally-ho:latest
```

## Schema changes

```
cargo run --features ssr --bin migrate -- migration generate --name what-changed
```

Migrations are applied at startup.

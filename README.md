<p align="center">
  <img src="public/icon-192.png" width="72" alt="">
</p>

<h1 align="center">tally-ho</h1>

Photograph a receipt, a local vision model pulls the line items out of it, then
upload your card statement and reconcile it charge by charge. Each person in Settings
has a description of what they tend to buy, and that's what decides who pays what.
A final CSV can be exported with the approved reconciliations.

Leptos and axum, SQLite through toasty, extraction via llama.cpp.

## Development

```
nix develop          # or let direnv do it
cargo leptos watch
```

Needs a llama-server serving both models by name (router mode): one to read a
receipt photo, one to turn the text into line items.

| Variable | Default |
|---|---|
| `LLAMA_URL` | `http://localhost:8080` |
| `LLAMA_MODEL` | `gemma4:12b` |
| `LLAMA_OCR_MODEL` | `glm-ocr:q8_0` — empty to let `LLAMA_MODEL` read the photo itself |
| `LLAMA_ASSIGN_MODEL` | same as `LLAMA_MODEL` — the one that guesses whose each item is |
| `MAX_IMAGE_EDGE` | `1600` — longest edge a photo is downscaled to before it's read |
| `CURRENCY` | `CAD` — the ISO code your statements are in and the default receipt currency |
| `DATABASE_URL` | `sqlite:./data/tally-ho.db` |
| `DATA_DIR` | `./data` |

## Container

```
nix build .#image
nix run .#image.copyToDockerDaemon
docker run -p 3000:3000 -v tally-ho:/data \
  -e LLAMA_URL=http://your-host:8080 tally-ho:latest
```

## Schema changes

```
cargo run --features ssr --bin migrate -- migration generate --name what-changed
```

Migrations are applied at startup.

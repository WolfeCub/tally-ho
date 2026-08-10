<p align="center">
  <img src="public/icon-192.png" width="72" alt="">
</p>

<h1 align="center">tally-ho</h1>

Photograph a receipt, a local vision model pulls the line items out of it, then
upload your card statement and reconcile it charge by charge — every one matched
to a receipt or explained — and export who owes what as CSV.

Leptos and axum, SQLite through toasty, extraction via Ollama.

## Development

```
nix develop          # or let direnv do it
cargo leptos watch
```

Needs an Ollama instance with a vision model pulled.

| Variable | Default |
|---|---|
| `OLLAMA_URL` | `http://localhost:11434` |
| `OLLAMA_VISION_MODEL` | `gemma4:12b` |
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

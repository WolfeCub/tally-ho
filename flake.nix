{
  description = "tally-ho — receipt capture and credit-card reconciliation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    rust-overlay,
  }:
    utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        # wasm32 target is required for the Leptos client bundle; nixpkgs'
        # plain `rustc` does not ship it.
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src" "rust-analyzer"];
          targets = ["wasm32-unknown-unknown"];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.cargo-leptos

            # nixpkgs builds cargo-leptos with the `no_downloads` feature, so
            # it will NOT fetch these itself — it resolves each with `which` and
            # hard-errors if absent (its src/ext/exe.rs). All three must be here.
            pkgs.tailwindcss_4 # provides bin/tailwindcss
            pkgs.binaryen # provides wasm-opt

            # Must match the `wasm-bindgen` crate version pinned in Cargo.toml
            # exactly, or wasm-bindgen fails with a version-mismatch error. The
            # unversioned `wasm-bindgen-cli` attr tracks a different version.
            pkgs.wasm-bindgen-cli_0_2_126

            pkgs.sqlite
            pkgs.bacon
            pkgs.pre-commit
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          # App defaults, so `cargo leptos watch` needs no env prefix. Override
          # any of them inline: `OLLAMA_VISION_MODEL=qwen3-vl:8b cargo leptos watch`.
          DATABASE_URL = "sqlite:./data/tally-ho.db";
          DATA_DIR = "./data";
          # Ollama runs on the OrbStack host, not in this container.
          OLLAMA_URL = "http://host.orb.internal:11434";
          OLLAMA_VISION_MODEL = "gemma4:12b";
          # 1024 is what the extraction was tuned and measured against. Image
          # size barely affects latency (prefill is ~1s at any size), so there
          # is little to gain by lowering it further.
          MAX_IMAGE_EDGE = "1024";
          RUST_LOG = "info,tally_ho=debug";
        };

        # TODO: `packages.default` (and therefore CI's `nix build`) is not yet
        # defined. naersk cannot build this crate: it has no default features,
        # so the lib compiles with neither `ssr` nor `hydrate`, and the client
        # wasm bundle needs cargo-leptos rather than plain cargo. Revisit when
        # switching to toasty migrations.
      }
    );
}

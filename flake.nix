{
  description = "tally-ho — receipt capture and credit-card reconciliation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    nix2container.url = "github:nlewo/nix2container";
    nix2container.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    rust-overlay,
    crane,
    nix2container,
    ...
  }:
    utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        # nixpkgs' rustc has no wasm32 target, which the client bundle needs.
        # minimal, since default includes 880MB of HTML rust docs
        rustToolchain = pkgs.rust-bin.stable.latest.minimal.override {
          extensions = ["rust-src" "rust-analyzer" "clippy" "rustfmt"];
          targets = ["wasm32-unknown-unknown"];
        };

        # cargo-leptos here is built with no_downloads, so it won't fetch any of
        # these itself. A missing one is a hard error.
        leptosTools = [
          pkgs.cargo-leptos
          pkgs.tailwindcss_4
          pkgs.binaryen # wasm-opt
          # Keep in sync with the wasm-bindgen pin in Cargo.toml.
          pkgs.wasm-bindgen-cli_0_2_126
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Allowlist, so touching flake.nix or CI doesn't rebuild the app.
        # filterCargoSources covers *.rs, *.toml and Cargo.lock.
        src = pkgs.lib.cleanSourceWith {
          name = "tally-ho-src";
          src = ./.;
          filter = path: type:
            craneLib.filterCargoSources path type
            || pkgs.lib.hasSuffix ".css" path
            || pkgs.lib.hasSuffix ".sql" path
            # Tailwind reads this to decide what to scan. Without it it walks
            # target/ and pulls classes out of our deps.
            || pkgs.lib.hasSuffix "/.gitignore" path
            # Whole tree — PWA icons and manifest go here.
            || pkgs.lib.hasInfix "/public/" path;
        };

        common = {
          inherit src;
          strictDeps = true;
          nativeBuildInputs = leptosTools;

          # cargo fingerprints this into leptos_config, so every cargo run here
          # has to agree or that crate and the five below it rebuild. Keep it
          # matching output-name.
          LEPTOS_OUTPUT_NAME = "tally_ho";
        };

        tally-ho = craneLib.buildPackage (common
          // {
            # crane primes one target, cargo leptos builds two. The client half
            # has its own target dir, triple and profile, so without this its
            # ~190 deps rebuild every time. Flags have to match cargo leptos'.
            cargoArtifacts = craneLib.buildDepsOnly (common
              // {
                cargoExtraArgs = "--locked --features ssr";
                postBuild = ''
                  cargo build --locked --features hydrate --lib \
                    --profile wasm-release \
                    --target wasm32-unknown-unknown \
                    --target-dir target/front
                '';
              });

            buildPhaseCargoCommand = "cargo leptos build --release";
            # Plain cargo test, so a build also fails when the models have
            # drifted from the migrations.
            cargoTestCommand = "cargo test --release --features ssr";

            nativeBuildInputs = common.nativeBuildInputs ++ [pkgs.makeWrapper];

            # crane finds binaries by parsing a cargo JSON build log. cargo leptos
            # writes none, and the hook is fatal, so the build dies without this.
            doNotPostBuildInstallCargoBinaries = true;

            # The default fixup only strips debug sections, and a rust release
            # build has none — the symbol table is the 16MB. Opt in to a full
            # strip, which is per package in nixpkgs.
            stripAllList = ["bin"];

            # cargo-leptos 0.3.7 puts the binary in target/release. Only
            # bin-target gets built, so there's no `migrate` to install.
            installPhaseCommand = ''
              mkdir -p $out/bin $out/share/tally-ho
              cp -r target/site toasty $out/share/tally-ho/
              install -Dm755 target/release/tally-ho $out/bin/tally-ho

              # These default to cwd-relative paths, which breaks once installed.
              wrapProgram $out/bin/tally-ho \
                --set-default LEPTOS_SITE_ROOT $out/share/tally-ho/site \
                --set-default LEPTOS_ENV PROD \
                --set-default MIGRATIONS_DIR $out/share/tally-ho/toasty
            '';

            meta.mainProgram = "tally-ho";
          });

        # Nothing references the flake inputs once evaluation is done, so the GC
        # takes them and the next CI run re-fetches nixpkgs. CI roots this.
        flake-inputs = pkgs.linkFarm "flake-inputs" self.inputs;

        # Output is a JSON manifest, not a tarball, so `docker load` won't read it.
        # Use `nix run .#image.copyToDockerDaemon`.
        image = nix2container.packages.${system}.nix2container.buildImage {
          name = "tally-ho";
          tag = "latest";
          # The default of 1 would re-push the whole closure for a one-line change.
          maxLayers = 100;

          config = {
            entrypoint = ["${tally-ho}/bin/tally-ho"];
            workingdir = "/data";
            exposedports."3000/tcp" = {};
            # Keeps the db and photos out of the writable layer without a -v.
            volumes."/data" = {};

            env = [
              "LEPTOS_SITE_ADDR=0.0.0.0:3000"
              "DATA_DIR=/data"
              "DATABASE_URL=sqlite:/data/tally-ho.db"
              # reqwest won't build a client without a CA bundle, and it panics
              # rather than returning an error. There's no /etc/ssl in the image.
              "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              # Without a zoneinfo db, jiff ignores TZ and uses UTC, which can file
              # a receipt in the wrong statement period.
              "TZDIR=${pkgs.tzdata}/share/zoneinfo"
              "RUST_LOG=info"
            ];
          };
        };
      in {
        packages = {
          default = image;
          inherit image tally-ho flake-inputs;
        };

        devShells.default = pkgs.mkShell {
          packages =
            [
              rustToolchain
              pkgs.sqlite
              pkgs.bacon
              pkgs.pre-commit
            ]
            ++ leptosTools;

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          # So `cargo leptos watch` needs no env prefix.
          DATABASE_URL = "sqlite:./data/tally-ho.db";
          DATA_DIR = "./data";
          # Ollama runs on the OrbStack host, not in here.
          OLLAMA_URL = "http://host.orb.internal:11434";
          OLLAMA_VISION_MODEL = "gemma4:12b";
          OLLAMA_OCR_MODEL = "glm-ocr:q8_0";
          RUST_LOG = "info,tally_ho=debug";
        };
      }
    );
}

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
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src" "rust-analyzer"];
          targets = ["wasm32-unknown-unknown"];
        };

        # This cargo-leptos is built with no_downloads, so it won't fetch these.
        # A missing one is a hard error.
        leptosTools = [
          pkgs.cargo-leptos
          pkgs.tailwindcss_4
          pkgs.binaryen # wasm-opt
          # Keep in sync with the wasm-bindgen pin in Cargo.toml.
          pkgs.wasm-bindgen-cli_0_2_126
        ];

        # cargo leptos runs cargo twice, so any builder needs its build command
        # swapped out. crane makes that a plain argument.
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Allowlist, so touching flake.nix or CI doesn't rebuild the app.
        # filterCargoSources covers *.rs, *.toml and Cargo.lock
        # ./. filters to .gitignore since flakes are git based
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
          pname = "tally-ho";
          version = "0.1.0";
          strictDeps = true;
          nativeBuildInputs = leptosTools;
        };

        tally-ho = craneLib.buildPackage (common
          // {
            # ssr, since that's nearly all the deps.
            cargoArtifacts = craneLib.buildDepsOnly (common // {cargoExtraArgs = "--features ssr";});

            buildPhaseCargoCommand = "cargo leptos build --release";
            # Plain cargo test, not `cargo leptos test`. Means a build also fails if
            # the models have drifted from the migrations.
            cargoTestCommand = "cargo test --release --features ssr";

            nativeBuildInputs = common.nativeBuildInputs ++ [pkgs.makeWrapper];

            # crane finds binaries by parsing a cargo JSON build log. cargo leptos
            # writes none, and the hook is fatal, so the build dies without this.
            doNotPostBuildInstallCargoBinaries = true;

            # 0.3.7 puts the binary in target/release, not target/server/release like
            # older templates. Only bin-target is built, so no `migrate` here.
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

        # Output is a JSON manifest, not a tarball, so `docker load` won't read it.
        # Use `nix run .#image.copyToDockerDaemon`.
        image = nix2container.packages.${system}.nix2container.buildImage {
          name = "tally-ho";
          tag = "latest";
          # The default of 1 would re-push the whole closure for a one-line change.
          maxLayers = 100;

          copyToRoot = [
            tally-ho
            # Without a zoneinfo db, jiff ignores TZ and uses UTC, which can file a
            # receipt in the wrong statement period.
            pkgs.tzdata
          ];

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
              "TZDIR=${pkgs.tzdata}/share/zoneinfo"
              "RUST_LOG=info"
            ];
          };
        };
      in {
        packages = {
          default = image;
          inherit image tally-ho;
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

          # So `cargo leptos watch` needs no env prefix. Override inline:
          # `OLLAMA_VISION_MODEL=qwen3-vl:8b cargo leptos watch`.
          DATABASE_URL = "sqlite:./data/tally-ho.db";
          DATA_DIR = "./data";
          # Ollama runs on the OrbStack host, not in here.
          OLLAMA_URL = "http://host.orb.internal:11434";
          OLLAMA_VISION_MODEL = "gemma4:12b";
          # What extraction was tuned against. Going lower doesn't help; prefill is
          # ~1s at any size.
          MAX_IMAGE_EDGE = "1024";
          RUST_LOG = "info,tally_ho=debug";
        };
      }
    );
}

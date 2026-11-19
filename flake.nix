{
  description = "osm_search_core — standalone OSM geocoding/search core (Rust cdylib). Reads an OSM region PBF, builds a per-region SQLite FTS5+rtree index, answers geocode/reverse-geocode offline. No module dependencies; the osm_search app passes catalog/cache_dir/pbf as invoke_op args.";

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";
    nixpkgs.follows = "logos-module-builder/nixpkgs";
    # Provides logos-lidl-gen (the contract->scaffold generator the builder
    # runs) and the SDK the crate links. One extra input vs a C++ module.
    logos-rust-sdk.url = "github:logos-co/logos-rust-sdk/49dbb0768a9a18cfaf264cc3ecfa55cca2630157";
    logos-rust-sdk.inputs.logos-module-builder.follows = "logos-module-builder";
  };

  outputs = inputs@{ self, logos-module-builder, ... }:
    let
      nixpkgs = logos-module-builder.inputs.nixpkgs;
      # x86_64-windows is a mingw cross-build realised on a Linux runner;
      # the builder bakes its pinned logos-nix into lib, so mkLogosModule
      # already produces a packages.x86_64-windows attribute.
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" "x86_64-windows" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
    in {
      packages = forAllSystems (system:
        (logos-module-builder.lib.mkLogosModule {
          src = ./.;
          configFile = ./metadata.json;
          flakeInputs = inputs;
        }).packages.${system});
    };
}

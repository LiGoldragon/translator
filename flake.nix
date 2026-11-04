{
  description = "translator — v1 typed Signal and Datom boundary";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-build = { url = "github:LiGoldragon/rust-build"; inputs.nixpkgs.follows = "nixpkgs"; };
    ethos-zero.url = "github:LiGoldragon/ethos-zero";
  };
  outputs = { self, nixpkgs, flake-utils, rust-build, ethos-zero }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        rust = rust-build.lib.${system}.fromPkgs pkgs;
        source = rust.cleanSource { root = ./.; };
        cargoArtifacts = rust.craneLib.buildDepsOnly { src = source; };
      in { packages.default = rust.craneLib.buildPackage { src = source; inherit cargoArtifacts; };
           checks = {
             test = rust.craneLib.cargoTest { src = source; inherit cargoArtifacts; cargoTestExtraArgs = "--all-features"; };
             identity-generated = pkgs.runCommand "translator-identity-generated" {
               generator = ethos-zero.packages.${system}.default;
               declaration = ./identity.ethos;
               committed = ./generated/identity.rs;
             } ''
               temporary="$(mktemp -d)"
               "$generator/bin/ethos-zero" "Generate.{ $declaration $temporary }" | grep -q '^Generated'
               name="$(basename "$declaration" .ethos)"
               cmp "$temporary/$name.rs" "$committed"
               touch "$out"
             '';
             placement-generated = pkgs.runCommand "translator-placement-generated" {
               generator = ethos-zero.packages.${system}.default;
               declaration = ./placement.ethos;
               committed = ./generated/placement.rs;
             } ''
               temporary="$(mktemp -d)"
               "$generator/bin/ethos-zero" "Generate.{ $declaration $temporary }" | grep -q '^Generated'
               name="$(basename "$declaration" .ethos)"
               cmp "$temporary/$name.rs" "$committed"
               touch "$out"
             '';
           };
           devShells.default = pkgs.mkShell { packages = [ rust.toolchain ]; }; });
}

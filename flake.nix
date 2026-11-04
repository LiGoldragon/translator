{
  description = "translator — v1 typed Signal and Datom boundary";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-build = { url = "github:LiGoldragon/rust-build"; inputs.nixpkgs.follows = "nixpkgs"; };
  };
  outputs = { self, nixpkgs, flake-utils, rust-build }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = import nixpkgs { inherit system; }; rust = rust-build.lib.${system}.fromPkgs pkgs;
      in { packages.default = rust.craneLib.buildPackage { src = rust.cleanSource { root = ./.; }; };
           checks.test = rust.craneLib.cargoTest { src = rust.cleanSource { root = ./.; }; cargoTestExtraArgs = "--all-features"; };
           devShells.default = pkgs.mkShell { packages = [ rust.toolchain ]; }; });
}

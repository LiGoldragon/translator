{
  description = "translator — v1 typed Signal and Datom boundary";

  inputs = {
    base.url = "github:LiGoldragon/build-base";
    ethos-zero = {
      url = "github:LiGoldragon/ethos-zero";
      inputs.base.follows = "base";
    };
  };

  # Cargo.nix is generated from Cargo.lock by the build base's crate2nix and
  # committed, so evaluation imports no derivation:
  #   nix shell github:LiGoldragon/build-base#toolchain github:LiGoldragon/build-base#crate2nix -c crate2nix generate
  outputs =
    { self, base, ethos-zero }:
    let
      lib = base.inputs.nixpkgs.lib;
      arguments = {
        cargoNix = ./Cargo.nix;

        # Every test runs with every feature on.
        memberChecks =
          { check, ... }:
          {
            translator = check "translator" [ "default" "datom" "text" "unix" ];
          };

        extraChecks =
          { pkgs, check, doc, ... }:
          let
            generator = ethos-zero.packages.${pkgs.stdenv.hostPlatform.system}.default;
            generated = name: declaration: committed:
              pkgs.runCommand "translator-${name}-generated" { inherit generator declaration committed; } ''
                temporary="$(mktemp -d)"
                "$generator/bin/ethos-zero" "Generate.{ $declaration $temporary }" | grep -q '^Generated'
                cmp "$temporary/$(basename "$declaration" .ethos).rs" "$committed"
                touch "$out"
              '';
          in
          {
            # The placement tests under the default feature set alone.
            test-default-placement = check "translator" [ "default" ];
            identity-generated = generated "identity" ./identity.ethos ./generated/identity.rs;
            placement-generated = generated "placement" ./placement.ethos ./generated/placement.rs;
            doc = doc "translator" [ "default" "datom" "text" "unix" ] [ "-D" "warnings" ];
            # A place file made by mkPlaceFile, the tool run over a datom.
            place-file = self.lib.${pkgs.stdenv.hostPlatform.system}.mkPlaceFile {
              placeName = "main";
              runtime = "/run/user/1000/nexus/main";
              state = "/home/example/.local/state/nexus/main";
            };
          };
      };
      workspaces = lib.genAttrs base.lib.systems (system: base.lib.workspace.${system} arguments);
    in
    {
      packages = lib.mapAttrs (
        _: workspace:
        workspace.packages
        // {
          # nexus-configure, the one tool that compiles a place written in
          # datom into the place file every Nexus reads at start. Its
          # features are the build base's datom set, so the library it links
          # is the one a Nexus taking translator with text and unix links.
          nexus-configure = workspace.binaries "translator" [
            "default"
            "text"
            "unix"
          ];
        }
      ) workspaces;

      checks = lib.mapAttrs (_: workspace: workspace.checks) workspaces;

      devShells = lib.mapAttrs (_: workspace: { default = workspace.devShell; }) workspaces;

      lib = lib.genAttrs base.lib.systems (system: {
        # A Nexus's place file, its own derivation: nexus-configure run over
        # the place written in datom. The file is the output itself; a home
        # links it at nexus/<placeName>/<nexus-name>.signal.
        #   mkPlaceFile { placeName = "main"; runtime = "/run/user/1001/nexus/main";
        #                 state = "/home/li/.local/state/nexus/main"; }
        mkPlaceFile =
          {
            placeName,
            runtime,
            state,
          }:
          let
            pkgs = base.lib.contexts.${system}.pkgs;
            datom = pkgs.writeText "nexus-place-${placeName}.datom" ''
              { «${placeName}» «${runtime}» «${state}» }
            '';
          in
          pkgs.runCommand "nexus-place-${placeName}.signal" { } ''
            ${self.packages.${system}.nexus-configure}/bin/nexus-configure \
              "Compile.{ «${datom}» «$PWD/place.signal» }"
            cp place.signal "$out"
          '';
      });
    };
}

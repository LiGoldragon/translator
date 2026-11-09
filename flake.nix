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
    base.lib.workspaceOutputs {
      cargoNix = ./Cargo.nix;

      # Every test runs with every feature on.
      memberChecks =
        { check, ... }:
        {
          translator = check "translator" [ "default" "datom" "text" "unix" ];
        };

      extraChecks =
        { pkgs, check, doc, clippy, ... }:
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
          clippy = clippy "translator" [ "default" "datom" "text" "unix" ] [ "-D" "warnings" ];
          doc = doc "translator" [ "default" "datom" "text" "unix" ] [ "-D" "warnings" ];
        };
    };
}

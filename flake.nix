{
  description = "translator — v1 typed Signal and Datom boundary, and the text lanes and pair launcher every translator shares";

  inputs = {
    base.url = "github:LiGoldragon/build-base";
    # The generator that regenerates each committed Ethos generation for its
    # check: 30.0.1, the one the nexus crates are generated with.
    ethos-zero = {
      url = "github:LiGoldragon/ethos-zero/2999292dc67c8608e54d32c333c550b29153cbff";
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
          translator-client = check "translator-client" [ "default" "datom" ];
          translator-serving = check "translator-serving" [ "default" "datom" "text" ];
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
          awaiting-generated = generated "awaiting" ./awaiting.ethos ./generated/awaiting.rs;
          client-generated = generated "client" ./crates/client/ethos/client.ethos ./crates/client/src/generated/client.rs;
          serving-generated = generated "serving" ./crates/serving/ethos/serving.ethos ./crates/serving/src/generated/serving.rs;
          doc = doc "translator" [ "default" "datom" "text" "unix" ] [ "-D" "warnings" ];
          doc-translator-client = doc "translator-client" [ "default" "datom" ] [ "-D" "warnings" ];
          doc-translator-serving = doc "translator-serving" [ "default" "datom" "text" ] [ "-D" "warnings" ];
        };
    };
}

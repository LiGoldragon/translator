{
  description = "The translator library, version 1, and the Relay Nexus that shows it.";

  inputs.base.url = "github:LiGoldragon/build-base";

  # ethos-zero, at the revision that generates a Nexus from all its roots at once, checks the ethos of
  # this repository and regenerates the Relay Nexus's contract from it.
  inputs.ethos-zero.url = "github:LiGoldragon/ethos-zero/6e6799526d9b9860834dc51ff77fe7a02af09fb9";
  inputs.ethos-zero.inputs.base.follows = "base";

  outputs = { self, base, ethos-zero }: base.lib.workspaceOutputs {
    cargoNix = ./Cargo.nix;
    extraChecks = { pkgs, ... }:
      let
        ethosZero = "${ethos-zero.packages.${pkgs.stdenv.hostPlatform.system}.ethos-zero}/bin/ethos-zero";
      in
      {
        # Every ethos file is checked, and the Relay Nexus's generated contract is what its ethos generates:
        # a Nexus's roots are checked and generated together, as one set, each module written as <stem>.rs.
        ethos = pkgs.runCommand "ethos" { } ''
          set -eu
          expect() {
            reply="$(${ethosZero} "$1")"
            case "$reply" in Checked.*|Generated.*) ;; *) echo "$1: $reply" >&2; exit 1 ;; esac
          }
          mkdir relay generated
          cp ${./relay-signal/ethos/signal.ethos} relay/signal.ethos
          cp ${./relay-signal/ethos/emission.ethos} relay/emission.ethos
          expect "Check.[ ${./transport/ethos/library.ethos} ]"
          expect "Check.[ ${./translator/ethos/library.ethos} ]"
          expect "Check.[ $PWD/relay/signal.ethos $PWD/relay/emission.ethos ]"
          expect "Generate.{ [ $PWD/relay/signal.ethos $PWD/relay/emission.ethos ] $PWD/generated }"
          diff generated/signal.rs ${./relay-signal/src/generated/signal.rs}
          diff generated/emission.rs ${./relay-signal/src/generated/emission.rs}
          touch "$out"
        '';
      };
  };
}

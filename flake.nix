{
  description = "The translator library, version 1, and the Relay Nexus that shows it.";

  inputs.base.url = "github:LiGoldragon/build-base";

  # ethos-zero, at the revision that reads the Process root, checks the ethos of
  # this repository and regenerates the Relay Nexus's contract from it.
  inputs.ethos-zero.url = "github:LiGoldragon/ethos-zero/48f155da487684cbebddf8e1fff6d6e8453abc9c";
  inputs.ethos-zero.inputs.base.follows = "base";

  outputs = { self, base, ethos-zero }: base.lib.workspaceOutputs {
    cargoNix = ./Cargo.nix;
    extraChecks = { pkgs, ... }:
      let
        ethosZero = "${ethos-zero.packages.${pkgs.stdenv.hostPlatform.system}.ethos-zero}/bin/ethos-zero";
      in
      {
        # Every ethos file is checked, and the Relay Nexus's generated contract is what its ethos generates.
        ethos = pkgs.runCommand "ethos" { } ''
          set -eu
          for file in ${./transport/ethos/library.ethos} ${./translator/ethos/library.ethos} \
                      ${./relay-signal/ethos/signal.ethos} ${./relay-signal/ethos/emission.ethos}; do
            reply="$(${ethosZero} "Check.$file")"
            case "$reply" in Checked.*) ;; *) echo "$file: $reply" >&2; exit 1 ;; esac
          done
          mkdir generated
          for name in signal emission; do
            case "$name" in
              signal) file=${./relay-signal/ethos/signal.ethos} ;;
              emission) file=${./relay-signal/ethos/emission.ethos} ;;
            esac
            cp "$file" "$name.ethos"
            ${ethosZero} "Generate.{ $PWD/$name.ethos $PWD/generated }" > /dev/null
          done
          diff generated/signal.rs ${./relay-signal/src/generated/signal.rs}
          diff generated/emission.rs ${./relay-signal/src/generated/emission.rs}
          touch "$out"
        '';
      };
  };
}

{
  description = "boatsched: sailing-camp boat scheduler (Rust + OR-Tools CP-SAT + Preact SPA)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      packages.${system} = rec {
        web = pkgs.callPackage ./nix/web.nix { };
        boatsched = pkgs.callPackage ./nix/package.nix { inherit web; };
        default = boatsched;
        # The on-box gated cutover run by deploy/deploy.sh.
        boatsched-switch = pkgs.writeShellApplication {
          name = "boatsched-switch";
          runtimeInputs = with pkgs; [ coreutils curl jq sqlite nix ];
          text = builtins.readFile ./nix/switch.sh;
        };
      };

      checks.${system} = { inherit (self.packages.${system}) boatsched boatsched-switch; };

      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ self.packages.${system}.boatsched ];
        packages = [ pkgs.nodejs_22 pkgs.sqlite pkgs.rust-analyzer ];
        ORTOOLS_ROOT = "${pkgs.or-tools}";
      };
    };
}

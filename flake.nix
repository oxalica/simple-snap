rec {
  description = "Minimalist BTRFS periodic snapshot tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      eachSystem = lib.genAttrs (lib.filter (lib.hasSuffix "-linux") lib.systems.flakeExposed);
    in
    {
      packages = eachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          rev = self.rev;
        in
        rec {
          default = simple-snap;
          simple-snap =
            with pkgs;
            rustPlatform.buildRustPackage {
              pname = "simple-snap";
              version = "git-${rev}";
              src = self;

              cargoLock.lockFile = ./Cargo.lock;

              meta = {
                inherit description;
                homepage = "https://github.com/oxalica/simple-snap";
                mainProgram = "simple-snap";
                license = [ lib.licenses.mit ];
                platforms = lib.platforms.linux;
              };
            };
        }
      );
    };
}

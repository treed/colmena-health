{
  inputs = {
    naersk.url = "github:nmattia/naersk/master";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-21.11";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, naersk, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        naersk-lib = pkgs.callPackage naersk { };
        libPath = with pkgs; lib.makeLibraryPath [
          openssl
        ];
      in
      {
        packages.default = naersk-lib.buildPackage {
          src = ./.;
          doCheck = true;
          pname = "colmena-health";
          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
          ];
          buildInputs = with pkgs; [
            openssl
          ];
          postInstall = ''
            wrapProgram "$out/bin/colmena-health" --prefix LD_LIBRARY_PATH : "${libPath}"
          '';
        };

        apps.default = utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = with pkgs; mkShell {
          nativeBuildInputs = [
            pkg-config
          ];
          buildInputs = [
            cargo
            cargo-insta
            pre-commit
            rust-analyzer
            rustPackages.clippy
            rustc
            rustfmt
            openssh

            openssl
          ] ++ lib.optionals pkgs.stdenv.isDarwin [
            libiconv
            darwin.apple_sdk.frameworks.Security
          ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
          LD_LIBRARY_PATH = libPath;
          # GIT_EXTERNAL_DIFF = "${difftastic}/bin/difft";
        };
      });
}

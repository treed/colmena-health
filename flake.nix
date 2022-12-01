{
  inputs = {
    naersk.url = "github:nmattia/naersk/master";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-22.05";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, naersk, ... }:
    {
      nixosModules.healthcheckOptions = import ./options.nix;

      lib = let
        lib = nixpkgs.lib;
      in {
        injectHostname = hostname: _node: checkDef:
          lib.attrsets.recursiveUpdate {
            labels.hostname = hostname;
          }
            checkDef;

        mkChecks = let
          doMods = mods: hostname: node: checkDef: lib.lists.foldl (x: f: f x) checkDef (builtins.map (m: m hostname node) mods);
          getChecksFromNode = node: builtins.map (lib.attrsets.filterAttrsRecursive (n: v: v != null)) node.config.deployment.healthchecks;
        in
          mods: nodes: lib.lists.flatten (lib.attrsets.mapAttrsToList (n: v: builtins.map (doMods mods n v) (getChecksFromNode v)) nodes);

        mkApp = system: checks: let
          pkgs = nixpkgs.legacyPackages.${system};
          checker = "${self.packages.${system}.default}/bin/colmena-health";
          checkJson = pkgs.writeText "checks.json" (builtins.toJSON checks);
          checkScript = pkgs.writeScriptBin "healthcheck.sh" "exec ${checker} ${checkJson} $@";
        in
          utils.lib.mkApp {drv = checkScript;};
      };
    }
    //
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

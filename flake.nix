{
  inputs = {
    crane = {
      url = "github:ipetkov/crane/v0.15.1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      crane,
      nixpkgs,
      utils,
      ...
    }:
    {
      nixosModules.healthcheckOptions = import ./options.nix;

      lib =
        let
          lib = nixpkgs.lib;
        in
        {
          injectHostname =
            hostname: _node: checkDef:
            lib.attrsets.recursiveUpdate { labels.hostname = hostname; } checkDef;

          mkChecks =
            let
              doMods =
                mods: hostname: node: checkDef:
                lib.lists.foldl (x: f: f x) checkDef (builtins.map (m: m hostname node) mods);
              getChecksFromNode =
                node:
                builtins.map (lib.attrsets.filterAttrsRecursive (
                  n: v: v != null
                )) node.config.deployment.healthchecks;
            in
            mods: nodes:
            lib.lists.flatten (
              lib.attrsets.mapAttrsToList (n: v: builtins.map (doMods mods n v) (getChecksFromNode v)) nodes
            );

          mkApp =
            system: checks:
            let
              pkgs = nixpkgs.legacyPackages.${system};
              checker = "${self.packages.${system}.default}/bin/colmena-health";
              checkJson = pkgs.writeText "checks.json" (builtins.toJSON checks);
              checkScript = pkgs.writeShellScriptBin "healthcheck.sh" "exec ${checker} ${checkJson} $@";
            in
            utils.lib.mkApp { drv = checkScript; };
        };
    }
    // utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.lib.${system};

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          pname = "colmena-health";
          version = "0.1.0";

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs =
            with pkgs;
            [ openssl ]
            ++ lib.optionals pkgs.stdenv.isDarwin [
              libiconv
              darwin.apple_sdk.frameworks.Security
            ];
        };

        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { });
        colmena-health = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;

            doCheck = true;
          }
        );
      in
      {
        packages.default = colmena-health;

        apps.default = utils.lib.mkApp { drv = self.packages.${system}.default; };

        devShells.default =
          with pkgs;
          mkShell {
            nativeBuildInputs = [ pkg-config ];
            buildInputs =
              [
                cargo
                rust-analyzer
                rustPackages.clippy
                rustc
                rustfmt
                rusty-man

                openssl
              ]
              ++ lib.optionals pkgs.stdenv.isDarwin [
                libiconv
                darwin.apple_sdk.frameworks.Security
              ];
          };

        formatter = pkgs.nixfmt-rfc-style;

        checks = {
          inherit colmena-health;
        };
      }
    )
    // {
      # These only run on NixOS anyway
      hydraJobs.x86_64-linux.tests =
        let
          pkgs = nixpkgs.legacyPackages.x86_64-linux;
          checker = "${self.packages.x86_64-linux.default}/bin/colmena-health";
          dns = import ./nixos-tests/dns.nix { inherit pkgs checker; };
          http = import ./nixos-tests/http.nix { inherit pkgs checker; };
          ssh = import ./nixos-tests/ssh.nix { inherit pkgs checker; };
        in
        {
          inherit dns http ssh;
          all =
            pkgs.runCommand "all tests"
              {
                buildInputs = [
                  dns
                  http
                  ssh
                ];
              }
              ''
                touch $out
              '';
        };
    };
}

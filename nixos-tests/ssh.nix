{
  pkgs,
  checker,
  ...
}: let
  # taken from https://github.com/NixOS/nixpkgs/blob/master/nixos/tests/ssh-keys.nix
  snakeOilPrivateKey = pkgs.writeText "privkey.snakeoil" ''
    -----BEGIN EC PRIVATE KEY-----
    MHcCAQEEIHQf/khLvYrQ8IOika5yqtWvI0oquHlpRLTZiJy5dRJmoAoGCCqGSM49
    AwEHoUQDQgAEKF0DYGbBwbj06tA3fd/+yP44cvmwmHBWXZCKbS+RQlAKvLXMWkpN
    r1lwMyJZoSGgBHoUahoYjTh9/sJL7XLJtA==
    -----END EC PRIVATE KEY-----
  '';
  snakeOilPublicKey = pkgs.lib.concatStrings [
    "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHA"
    "yNTYAAABBBChdA2BmwcG49OrQN33f/sj+OHL5sJhwVl2Qim0vkUJQCry1zFpKTa"
    "9ZcDMiWaEhoAR6FGoaGI04ff7CS+1yybQ= snakeoil"
  ];

  sshClientConfig = pkgs.writeText "ssh-config" ''
    Host *
    UserKnownHostsFile /dev/null
    StrictHostKeyChecking no
  '';

  successConfig =
    pkgs.writeText "success.json"
    (builtins.toJSON {
      checks = [
        {
          type = "ssh";
          params.command = "true";
        }
      ];
      defaults.ssh.hostname = "checked";
    });
  failureConfig =
    pkgs.writeText "success.json"
    (builtins.toJSON {
      checks = [
        {
          type = "ssh";
          params.command = "false";
          retryPolicy = {maxRetries = 0;};
        }
      ];
      defaults.ssh.hostname = "checked";
    });
in
  pkgs.nixosTest {
    name = "ssh";

    nodes.checker = {...}: {};
    nodes.checked = {...}: {
      services.openssh.enable = true;
      users.users.root.openssh.authorizedKeys.keys = [
        snakeOilPublicKey
      ];
    };

    testScript = ''
      start_all()
      checked.wait_for_unit("sshd.service")

      checker.succeed("mkdir ~/.ssh")
      checker.succeed("cat ${sshClientConfig} > ~/.ssh/config")
      checker.succeed("cat ${snakeOilPrivateKey} > ~/.ssh/id_ecdsa")
      checker.succeed("chmod 400 ~/.ssh/id_ecdsa")
      checker.succeed("ssh -v checked uptime",timeout=30)

      checker.succeed("${checker} ${successConfig}")
      checker.fail("${checker} ${failureConfig}")
    '';
  }

{
  pkgs,
  checker,
  ...
}: let
  testfile = pkgs.writeTextFile {
    name = "http-test-file";
    destination = "/hello.txt";
    text = "hi!";
  };
  successConfig =
    pkgs.writeText "success.json"
    (builtins.toJSON {
      checks = [
        {
          type = "http";
          params.url = "http://localhost/testing/hello.txt";
        }
      ];
    });
  failureConfig =
    pkgs.writeText "failure.json"
    (builtins.toJSON {
      checks = [
        {
          type = "http";
          params.domain = "http://localhost/testing/does-not-exist.txt";
          retryPolicy = {maxRetries = 0;};
        }
      ];
    });
in
  pkgs.nixosTest {
    name = "http";

    nodes.checker = {...}: {
      services.nginx.enable = true;
      services.nginx.virtualHosts."test-server" = {
        serverName = "localhost";
        locations = {
          "/testing/".alias = "${testfile}/";
        };
      };
    };

    testScript = ''
      start_all()
      checker.wait_for_unit("nginx.service")

      checker.succeed("${checker} ${successConfig}")
      checker.fail("${checker} ${failureConfig}")
    '';
  }

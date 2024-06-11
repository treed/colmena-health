{ pkgs, checker, ... }:
let
  testfile = pkgs.writeTextFile {
    name = "http-test-file";
    destination = "/hello.txt";
    text = "hi!";
  };
  successConfig = pkgs.writeText "success.json" (
    builtins.toJSON {
      checks = [
        {
          alertPolicy = {
            checkInterval = 300;
            recheckInterval = 30;
          };
          annotations = { };
          checkTimeout = 10;
          labels = {
            type = "http";
          };
          params.url = "http://localhost/testing/hello.txt";
          retryPolicy = {
            initial = 1.0;
            maxRetries = 0;
            multiplier = 1.1;
          };
          type = "http";
        }
      ];
    }
  );
  failureConfig = pkgs.writeText "failure.json" (
    builtins.toJSON {
      checks = [
        {
          alertPolicy = {
            checkInterval = 300;
            recheckInterval = 30;
          };
          annotations = { };
          checkTimeout = 10;
          labels = {
            type = "http";
          };
          params.url = "http://localhost/testing/does-not-exist.txt";
          retryPolicy = {
            initial = 1.0;
            maxRetries = 0;
            multiplier = 1.1;
          };
          type = "http";
        }
      ];
    }
  );
in
pkgs.nixosTest {
  name = "http";

  nodes.checker =
    { ... }:
    {
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

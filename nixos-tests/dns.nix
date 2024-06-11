{ pkgs, checker, ... }:
let
  testZoneFile = pkgs.writeText "test-zone" ''
    $ORIGIN colmena-health.test.
    @	600 IN	SOA sns.dns.icann.org. noc.dns.icann.org. 2021071801 7200 3600 1209600 3600
     	600 IN NS a.iana-servers.net.
     	600 IN NS b.iana-servers.net.

    test-host1     IN A     192.0.2.10
  '';
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
            type = "dns";
          };
          params.domain = "test-host1.colmena-health.test";
          retryPolicy = {
            initial = 1.0;
            maxRetries = 0;
            multiplier = 1.1;
          };
          type = "dns";
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
            type = "dns";
          };
          params.domain = "does-not-exist.colmena-health.test";
          retryPolicy = {
            initial = 1.0;
            maxRetries = 0;
            multiplier = 1.1;
          };
          type = "dns";
        }
      ];
    }
  );
in
pkgs.nixosTest {
  name = "dns";

  nodes.checker =
    { ... }:
    {
      services.coredns = {
        enable = true;
        config = ''
          colmena-health.test {
            bind lo eth0
            file ${testZoneFile}
            errors
          }
        '';
      };

      networking.nameservers = [ "127.0.0.1" ];
    };

  testScript = ''
    start_all()
    checker.wait_for_unit("coredns.service")
    checker.wait_until_succeeds("netstat -tulpen | grep coredns", timeout=30)

    checker.succeed("host test-host1.colmena-health.test")

    checker.succeed("${checker} ${successConfig}")
    checker.fail("${checker} ${failureConfig}")
  '';
}

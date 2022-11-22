# Colmena Health

This is a very simple prototype of a health checker that can tie in with [Colmena](https://github.com/zhaofengli/colmena).

It is not intended to live very long, and exists only as a proof of concept for how healthchecks might work.

The interface will probably change a fair bit as I determine how everything should work.

If this works out, my intention is to try to see this merged into Colmena itself, although as written it could technically work with any deployment tool, or even without one.

## Current State

As is, it is capable of three types of checks, which can be driven from a json config that can be gathered through `colmena eval`.

I added the following to my common module, to ensure that different modules could specify checks for the same host.

```nix
  options = {
    deployment.healthchecks = lib.mkOption {
      type = with lib.types; listOf anything;
      default = [];
    };
  };
```

I can then add definitions like the following to one of my modules:


```nix
    deployment.healthchecks = [
      { type = "http"; url = "http://${config.networking.hostName}.dc1.example.com:5050"; }
      { type = "http"; url = "http://${config.networking.hostName}.dc1.example.com:${toString config.services.my-service.port}"; }
      { type = "dns"; domain = "my-service.service.consul."; }
      { type = "http"; url = "https://my-service.example.com"; }
    ];
```

Which can then be run like this:

I currently have this in my flake to generate a list of healthchecks from that data:

```nix
      checks = let
        lib = nixpkgs.lib;
        removeKey = key: set: lib.attrsets.filterAttrs (n: v: n != key) set;
        mkCheck = hostname: def: {
          type = def.type;
          labels = {inherit hostname;};
          params =
            removeKey "type" def
            // (
              if def.type == "ssh"
              then {inherit hostname;}
              else {}
            );
        };
      in {
        checks = lib.lists.flatten (lib.attrsets.mapAttrsToList (n: v: builtins.map (mkCheck n) v.config.deployment.healthchecks) self.colmenaHive.nodes);
      };
```

It's a little complicated, but I intend to add a module and lib functions to this tool's flake to make it a bit simpler on the user's side.

This can give you output like the following:

```
[ssh 'svc-host1': systemctl is-active nginx] status update: Running
[http http://svc-host1.dc1.example.com:5050] status update: Running
[ssh 'svc-host1': systemctl is-active consul] status update: Running
[http http://svc-host1.dc1.example.com:5000] status update: Running
[http http://svc-host1.dc1.example.com:5050] status update: Succeeded
[ssh 'svc-host1': systemctl is-active consul] status update: Succeeded
[ssh 'svc-host1': systemctl is-active nginx] status update: Succeeded
[http http://svc-host1.dc1.example.com:5000] status update: Waiting after failure: Check timed out
[http http://svc-host1.dc1.example.com:5000] status update: Waiting after failure: Check timed out
[http http://svc-host1.dc1.example.com:5000] status update: Failed: Check timed out
1 check(s) failed
```

(In the future, this will likely be what verbose output looks like, and most of it will be hidden by default.)

## Configuration

The configuration file is JSON, with two top level keys: "checks" and "defaults".

Checks is a flat list of check definitions, each one with a full configuration for a specific check, given as an object.

The keys for a check definition are:

- type: The type of the check (`dns`, `http`, `ssh`)
- params: An object of parameters to pass to the check
- retry_policy: An object configuring how retries are handled
- labels: An object of arbitrary key/value data representing the check; this is used for selecting checks at run-time
- check_timeout: A number of seconds before each check iteration times out (defaults to 10)

The "defaults" key holds an object where the keys are either one of the check types, or retry_policy, and set the defaults for parameters not specified in the checks.

### HTTP Checks

HTTP checks will attempt to connect to a URL, and succeed if it is able to connect and the server responds with a successful status code.

It currently only has the one parameter, which is the URL to try.

```json
{
  "type": "http",
  "params": {
    "url": "http://github.com/treed/colmena-health"
  }
}
```

### DNS

DNS checks simply determine if a domain resolves at all. This will likely mostly be useful for DNS interfaces to service discovery, like Consul.

There is currently only the single `domain` parameter.

```json
{
  "type": "dns",
  "params": {
    "domain": "my-service.service.consul"
  }
}
```


### SSH

SSH checks will ssh to the target machine and run a command, failing unless ssh successfully connects and the remote command exits 0.

```json
{
  "type": "ssh",
  "params": {
    "command": "true",
	"hostname": "rack215-cl15",
	"username": "monitoring"
  }
}
```

It has three parameters:

- command (required): the command to run on the target
- hostname (required): the hostname of the target
- username (optional): the username to connect as

In the absence of a globally-set default, `username` defaults to `root`, in keeping with colmena`s default.

This implementation shells out to your `ssh` command for the simplicity in having full access to the user's own ssh config and agent.

Something perhaps worth calling out here is that the contents of commands won't necessarily be deployed to nodes without you doing it out-of-band. One way to handle this would be to use `pkgs.writeScript` to make a script-based package and ensure that's added to the system environment, and then use it as the command, which should have the correct store path after deployment.

### Retry Policy

A retry policy governs the use of retries during the check, and has three keys:

- max_retries (default 3): The number of retries before failing the check (set to 0 to disable retries entirely)
- initial (default 1): The number of seconds to wait before retrying
- multiplier (default 1.1): A multiplier to apply to the wait duration on each retry; applies exponential backoff

### Defaults

As mentioned above, the top level `defaults` key holds globally applied defaults for any check parameters not specified, or for unspecified retry_policy keys.

For check-type keys, the values are the same as the set of parameters for the check type.

For `retry_policy`, it's the same as a retry policy as you'd define in a check.

These can be used to override built-in defaults given above.

## TODO

### Flake Helpers

I want to add a module and lib functions to help generate the config file from within node configuration.

### More Check Configuration

Some check types could probably benefit from having more configuration (e.g. DNS record type and expected response, or HTTP expected status codes).

### Alert Configuration

I want to try having a long-running daemon mode that runs these checks at intervals, and sends failing checks to an alertmanager API for routing and delivery. (This is part of the reason for adding labels to check definitions.)

### Better Output

Right now, the output is purposefully very verbose. At some point, I want to default to a cleaner output mode that just shows a count of checks that are waiting, succeeded, or failed, and outputs more information for failures.

### More Check Types

Possibly something that will query Prometheus or Loki compatible endpoints?

### Check Dependencies

Sometimes it doesn't make sense to run some checks if some other check failed or at least hasn't succeeded yet, so it might be useful to have some way to indicate that some checks should only be run after other checks have succeeded.

I'm unsure about this one, and it would require some means of identifying checks (name or ID field), and an extra check to prevent circular dependencies.

### Code Cleanliness

The throw-it-together-quick nature shows some in the structure. I'll probably play with it some; in particular with seeing if I can only initialize the HTTP/DNS clients once rather than in each check.

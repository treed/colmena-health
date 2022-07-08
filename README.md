# Colmena Health

This is a very simple prototype of a health checker that can tie in with [Colmena](https://github.com/zhaofengli/colmena).

It is not intended to live very long, and exists only as a proof of concept for how healthchecks might work.

The interface will probably change a fair bit as I determine how everything should work.

If this works out, my intention is to try to see this merged into Colmena itself.

## Current State

As is, it is capable of two types of checks, which can be driven from a json config that can be gathered through `colmena eval`.

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

```sh
colmena eval -E '{ nodes, lib, ... }: { targets = lib.attrsets.mapAttrs (k: v: v.config.deployment.healthchecks) nodes; }' | colmena-health --on svc-host1 -
```

Which can give you output like the following:

```
[INFO ] Using configuration: /home/user/ops/cluster/hive.nix
Checking domain my-service.service.consul.' for 'svc-host1': Success
Checking url 'http://svc-host1.dc1.example.com:5050' for 'svc-host1': Success
Checking url 'http://svc-host1.dc1.example.com:5000' for 'svc-host1': Failed:
error sending request for url (http://svc-host1.dc1.example.com:5000/login/?redirect=%2F%3F): connection closed before message completed
Checking url 'https://my-service.example.com' for 'svc-host1': Success
Error: There was 1 failed check
```

### HTTP

HTTP checks will attempt to connect to a URL, and succeed if it is able to connect and the server responds with a successful status code.

```json
{ "type": "http", "url": "http://github.com/treed/colmena-health" }
```

It currently only has the one parameter, which is the URL to try.

There is a hardcoded timeout at five seconds.

### DNS

DNS checks simply determine if a domain resolves at all. This will likely mostly be useful for DNS interfaces to service discovery, like Consul.

```json
{ "type": "dns", "domain": "my-service.service.consul" }
```

There is currently only the single `domain` parameter.

### SSH

SSH checks will ssh to the target machine (by that name), and run a command, failing unless ssh successfully connects and the remote command exits 0.

```json
{ "type": "ssh", "command": "true" }
```

The single parameter `command` is the command to pass to the ssh client.

This implementation shells out to your `ssh` command for the simplicity in having full access to the user's own ssh config and agent.

## TODO

### More Check Types

Possibly something that will query Prometheus or Loki compatible endpoints?

### Better Check Retry/Timeout Control

Right now there's only one attempt. HTTP requests have a five second timeout, and the other two have no timeout at all.

I want all checks to have ways to control:

- number of retries
- retry interval (possibly with support for exponential backoff)
- initial delay
- per-attempt timeout

### Check Dependencies

Sometimes it doesn't make sense to run some checks if some other check failed or at least hasn't succeeded yet, so it might be useful to have some way to indicate that some checks should only be run after other checks have succeeded.

I'm unsure about this one, and it would require some means of identifying checks (name or ID field), and an extra check to prevent circular dependencies.

### Code Cleanliness

The throw-it-together-quick nature shows some in the structure. I'll probably play with it some; in particular with seeing if I can only initialize the HTTP/DNS clients once rather than in each check.

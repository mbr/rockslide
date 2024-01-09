# rockslide

`rockslide` is a simple, personal, self-hosted [Platform-as-service (PaaS)](https://en.wikipedia.org/wiki/Platform_as_a_service) that orchestrates [container images](https://en.wikipedia.org/wiki/Containerization_(computing)) as its core abstraction for shipping software. It allows a single developer to host one or more applications on a single machine with minimal fuss, simply pushing a container image to the built-in registry is enough to launch it on a custom domain with HTTPS support (hopefully soon) through letsencrypt.

## Features

* Single static binary less than 10 mb in size, with a tiny set of external dependencies (`podman`).
* No additional CLI tools needed, simply pushing to a registry is enough.
* Allows for optionally protecting containers with an HTTP password.

## Production readiness

The `rockslide` is derived from the being opposite of "rock solid", at this time, this software should be considered nothing more than a demo, especially since the TLS implementation is incomplete. Use it for low-value personal projects and out of curiosity only!

## Installation

To build and install, clone this repository and have [`nix`](https://nixos.org) installed. The contained `upload.sh` contains everything needed to deploy it to a new Linux system that has `podman` and `systemd` installed. The user is highly encouraged to read the short [`upload.sh`](upload.sh) and [`setup.sh`](etc/setup.sh) files first.

After installation, `/etc/rockslide.toml` can be edited, at the very least a `master_key` should be set, since the installation is rather uninteresting without one set.

It is highly recommended (though not necessary) to forward a wildcard DNS domain to the machine running `rockslide`, this documentation will use `*.rockslide.example.com` as a fictional instance of this.

## Running containers

With `podman` or `docker` installed on any local dev machine, we can pull an already existing "Hello, world" image, tag it and deploy it:

```
podman login --tls-verify=false http://rockslide.example.com
# (use any non-empty username and the `master_key` set earlier to login)
podman pull crccheck/hello-world
podman tag crccheck/hello-world rockslide.example.com/hi.rockslide.example.com/index:prod
podman push --tls-verify=false rockslide.example.com/hi.rockslide.example.com/index:prod
```

The `crccheck/hello-world` docker image will listen on any `PORT` (an environment variable automatically injected by `rockslide`) for incoming HTTP connections. Any docker image uploaded with the reference `:prod` will be made available under the server's root at `repo/image`. For example, tagging and uploading

```
podman tag myimage example.com/foo/bar:prod
podman push example.com/foo/bar:prod
```

means that the running container is reachable unter `http://example.com/foo/bar`.

Should the "repository" part (`foo`) look like a domain (`mydomain.com`) and the image part (`bar`) be exactly `index`, the server will also forward `mydomain.com` to the container. As an example, an image tagged `example.com/mydomain.com/index:prod` will be reachable both under `http://example.com/mydomain.com/index` as well as `http://mydomain.com`.

Note that `docker` could be used instead of `podman` for any of these commands, but disabling HTTPS is easier using `podman` at the moment (and necessary because of missing HTTPS support).

## Container runtime configuration

While configuration is mostly automatic, there is one feature that can optionally be configured: Password protection for containers.

In general, configuration per container is a single file that can be retrieved from the `_rockslide/config` subpath:

(this assumes an envvar `MASTER_KEY` is set with the master key configured during installation)

```
curl -u :$MASTER_KEY rockslide.example.com/_rockslide/config/hi.rockslide.example.com/index/prod > hi.toml
```

Note how `-u` is used for HTTP basic auth. Editing this file to require a user account and password of `foo:bar` is straightforward:

```toml
[http]
access = { foo = "bar" }
```

A single `PUT` request at the same location updates the access password.

```
curl -v -X PUT -u :$MASTER_KEY rockslide.example.com/_rockslide/config/hi.rockslide.example.com/index/prod --data-binar
y "@hi.toml"
```

## macOS suppport

macOS is supported as a tier 2 platform to develop rockslide itself, although currently completely untested for production use. [podman can run on Mac OS X](https://podman.io/docs/installation), where it will launch a Linux virtual machine to run containers. The `rockslide` application itself and its supporting nix-derivation all account for being built on macOS.

### Initializing `podman` using a qemu virtual machien

To run Linux containers on macOS, a background virtual machine is usually started and `podman` is setup to manage it remotely (`qemu` and `podman` are part of the `nix-shell` on macOS by default). This is done by running

```
podman machine init
podman machine start
```

You can verify podman is working correctly afterwards by running

```
podman run -it debian:latest /bin/sh -c 'echo everything is working fine'
```

`rockslide` will check an envvar `PODMAN_IS_REMOTE`, if it is `true`, it will assume a remote instance and act accordingly. This envvar is set to `true` automatically when running `nix-shell` on a macOS machine.

With these prerequisites fulfilled, `rockslide` should operate normally as it does on Linux.

# rockslide

## Container runtime configuration

```
curl -u :devpw localhost:3000/_rockslide/config/foo/bar/prod > foobarprod.toml
curl -v -X PUT -u :devpw localhost:3000/_rockslide/config/foo/bar/prod --data-binar
y "@foobarprod.toml"
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

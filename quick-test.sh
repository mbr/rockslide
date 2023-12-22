#!/bin/sh

podman pull crccheck/hello-world
podman tag  crccheck/hello-world 127.0.0.1:3000/testing/hello:prod
podman push --tls-verify=false 127.0.0.1:3000/testing/hello:prod

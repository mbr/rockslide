#!/bin/sh

set -e

if id rockslide >/dev/null 2>&1; then
  echo "rockslide user already setup, not recreating"
else
  groupadd -r rockslide
  useradd -m -g rockslide -d /var/lib/rockslide rockslide
  chmod 0700 /var/lib/rockslide
fi;

if [ ! -e /etc/rockslide.toml ]; then
  cp /etc/rockslide.example.toml /etc/rockslide.toml
  chown root:rockslide /etc/rockslide.toml
  chmod 0640 /etc/rockslide.toml
fi;

mkdir -p /var/lib/rockslide/registry
chown rockslide:rockslide /var/lib/rockslide
chown rockslide:rockslide /var/lib/rockslide/registry

systemctl daemon-reload
systemctl restart rockslide
systemctl enable rockslide

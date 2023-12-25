#!/bin/sh

#: This script can be used to install a compiled `rockslide` binary, which is statically linked,
#: onto a target server.
#:
#: Requirements:
#: * systemd
#: * podman installed
#: * /usr/local/bin and /etc/systemd/system writable

set -e

if [ $# -ne 1 ]; then
  echo usage: $(basename $0) TARGET_HOST
fi;

cd $(dirname $0)

TARGET=$1

scp result/bin/rockslide root@$TARGET:/usr/local/bin/rockslide
scp etc/rockslide.service root@$TARGET:/etc/systemd/system/rockslide.service
scp etc/setup.sh root@$TARGET:/root/
ssh root@$TARGET "/bin/sh /root/setup.sh"

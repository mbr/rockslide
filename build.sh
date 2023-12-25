#!/bin/sh

exec cargo build --release --offline --target=x86_64-unknown-linux-musl

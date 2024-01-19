# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2024-01-09

### Added

* Headers from external clients are passed through to running containers.
* The `X-Script-Name` header is set for path based reverse proxying.

### Changed

* Log messages have been cleaned up.
* The name separator in container names changed from `-` to `---` to allow dashes in domains.

### Fixed

* Images uploaded are now properly inspected (would cause all uploads to fail before).

## [0.1.0] - 2024-01-14

Initial release

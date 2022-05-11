# Changelog

This file documents recent notable changes to this project. The format of this
file is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and
this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2020-05-11

### Added

* `RequestCode` that identifies the type of request.

## Changed

* `recv_frame`, `recv_raw_frame`, and `recv_raw_frame` are now under `frame`;
  use `frame::recv`, `frame::recv_raw`, and `frame::recv_raw_frame` instead.

## [0.1.1] - 2022-05-10

### Added

* `recv_raw_frame` to receive length-delimited frames without deserializing
  them.

## [0.1.0] - 2022-05-09

### Added

* `send_frame` and `recv_frame` to send and receive length-delimited frames.

[0.1.1]: https://github.com/petabi/oinq/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/petabi/oinq/tree/0.1.0

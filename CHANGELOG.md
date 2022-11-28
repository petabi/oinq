# Changelog

This file documents recent notable changes to this project. The format of this
file is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and
this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2022-11-28

### Added

* Add `RequestCode::SamplingPolicyList` for updating sampling policies.
* Add `RequestCode::ReloadFilterRule` for updating traffic filtering rules of piglet.

### Removed

* `ResourceUsage` and functions related with getting resource usage.

## [0.5.0] - 2022-11-07

### Changed

* Requires Rust 1.64 or later.
* Requires quinn 0.9.0 or later.
* `server_handshake` takes `&Connection` instead of `&mut IncomingBiStreams`.

## [0.4.1] - 2022-08-24

### Added

* Add `RequestCode::TorExitNodeList` for updating tor exit nodes.

## [0.4.0] - 2022-06-26

### Added

* When the received request code is not recognized, the response code is set to
  `RequestCode::Unknown`.
* `message::send_request` and `message::send_forward_request` can accept any
  type implementing `Into<u32>` as `code`.

### Changed

* `RequestCode` no longer implements `Serialize` and `Deserialize`; it should be
  converted to/from `u32` for serialization/deserialization.
* `message::recv_request_raw` returns the request code as `u32` instead of
  `RequestCode`.
* A client sending an unknown request code will receive an error message.

## [0.3.0] - 2022-06-24

### Added

* `message::server_handshake` receives a handshake message and sends a response.
* `AgentInfo` implements `Display`.
* `AgentInfo::key` to obtain the agent's key.

### Changed

* Requires Rust 1.57.0 or later.
* `message::handshake` is now `message::client_handshake` to differentiate
  itself from the server handshake.
* `HandshakeError::NewerProtocolRequired` has been replaced with
  `HandshakeError::IncompatibleProtocol`, which carries both the current version
  and the required version.
* `HandhsakeError::StreamError` has been replaced with
  `HandshakeError::ConnectionLost`.

## [0.2.7] - 2022-06-08

### Added

* `messages::handshake` and the types it uses: `AgentInfo` and `HandshakeError`.
  It sends a handshake request and processes the response.

## [0.2.6] - 2022-05-27

### Added

* `request::Handler` defines the interface for a request handler for an agent.
* `request::handle` handles requests for an agent.

## [0.2.5] - 2022-05-23

### Added

* `send_ok` and `send_err` to send a response.

## [0.2.4] - 2022-05-18

### Added

* `handle_resource_usage` to handle the `ResourceUsage` request.

## [0.2.3] - 2022-05-17

### Added

* `RequestCode` supports serialization with serde.
* `recv_request_raw` to receive a request without deserialization.
* `send_request` to send a request with serialization.
* `send_forward_request` to send a `RequestCode::Forward` request.

## [0.2.2] - 2022-05-16

### Added

* `frame::send_raw` to send raw bytes in a frame.

## [0.2.1] - 2022-05-12

### Added

* `RequestCode::ReloadConfig` to request agent to reload configuration
* `RequestCode::ResourceUsage` to request agent to collect resource usage stats

## [0.2.0] - 2022-05-11

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

[0.6.0]: https://github.com/petabi/oinq/compare/0.5.0...0.6.0
[0.5.0]: https://github.com/petabi/oinq/compare/0.4.1...0.5.0
[0.4.1]: https://github.com/petabi/oinq/compare/0.4.0...0.4.1
[0.4.0]: https://github.com/petabi/oinq/compare/0.3.0...0.4.0
[0.3.0]: https://github.com/petabi/oinq/compare/0.2.7...0.3.0
[0.2.7]: https://github.com/petabi/oinq/compare/0.2.6...0.2.7
[0.2.6]: https://github.com/petabi/oinq/compare/0.2.5...0.2.6
[0.2.5]: https://github.com/petabi/oinq/compare/0.2.4...0.2.5
[0.2.4]: https://github.com/petabi/oinq/compare/0.2.3...0.2.4
[0.2.3]: https://github.com/petabi/oinq/compare/0.2.2...0.2.3
[0.2.2]: https://github.com/petabi/oinq/compare/0.2.1...0.2.2
[0.2.1]: https://github.com/petabi/oinq/compare/0.2.0...0.2.1
[0.2.0]: https://github.com/petabi/oinq/compare/0.1.1...0.2.0
[0.1.1]: https://github.com/petabi/oinq/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/petabi/oinq/tree/0.1.0

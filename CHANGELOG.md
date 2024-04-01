# Changelog

This file documents recent notable changes to this project. The format of this
file is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and
this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed

- `RequestCode::Forward` and `message::send_forward_request`, since forwarding
  messages between agents is no longer supported.
- `client_handshake`, `server_handshake`, and `AgentInfo`. These belong to the
  `review-protocol` crate.

## [0.11.0] - 2024-03-25

### Changed

- `request::Handler::update_traffic_filter_rules` takes a slice of `(IpNet,
  Option<Vec<u16>>, Option<Vec<u16>>)`, instead of `IpNet`, to support port
  numbers and protocols.

## [0.10.0] - 2024-02-23

### Changed

- Changed struct `Configuration` to enum `Configs` and defined configuration by
  module.

## [0.9.3] - 2024-02-16

### Added

- Add `RequestCode::Shutdown` to shutdown the host.

## [0.9.2] - 2023-12-07

### Added

- Add `RequestCode::SemiSupervisedModels` to update semi-supervised models of
  all Hogs.

## [0.9.1] - 2023-09-06

### Added

- `RequestCode::ProcessList` to request agent to collect host's process list.

### Fixed

- Fix clippy warnings.

## [0.9.0] - 2023-07-27

### Changed

- Change to respond with an immediate `Ok` message when receiving an `EchoRequest`
  message instead of calling the handler's function.

## [0.8.2] - 2023-06-15

### Added

- Introduced new request code: `RequestCode:TrustedUserAgentList`
  - `RequestCode::TrustedUserAgentList`: This new request code facilitates the
    REview  sending a list of trusted `user-agents` to the agent. This request code
    is followed by a `Vec<String>` parameter, which is a list of trusted user-agents.

## [0.8.1] - 2023-06-05

### Added

- New Request Code: `RequestCode:EchoRequest`. This addition to the request
code library is designed specifically for network latency testing, also
commonly referred to as ping testing. When a request is made to a daemon using
the `RequestCode::EchoRequest`, the daemon will respond with an empty tuple.
This allows users to measure round trip time between the client and the server
without relying on the content of the response.

## [0.8.0] - 2023-05-24

### Changed

- The transfer data types of the following request codes have been changed:

  - `RequestCode:InternalNetworkList`
  - `RequestCode:AllowList`
  - `RequestCode:BlockList`

  Prior to this update, these request codes were using a string array to
  transfer data. This approach has been replaced by the `HostNetworkGroup` data
  type.

  This change is backwards incompatible. Users are encouraged to update their
  request handling to accommodate the `HostNetworkGroup` data type.

## [0.7.1] - 2023-05-17

### Added

- Introduced two new request codes: `RequestCode::AllowList` and
  `RequestCode::BlockList`.
  - `RequestCode::AllowList`: This new request code facilitates REview to
    transmit the allow list to an agent. This request code is followed by a
    list of IP prefixes (represented as `Vec<String>`), which specifies the IP
    ranges allowed to communicate.
  - `RequestCode::BlockList`: This new request code enables REview to send the
    block list to an agent. This request code also follows a `Vec<String>`
    parameter which carries a list of IP prefixes that are to be blocked from
    communication.

  Each of these IP prefixes are in the format of "10.20.81.1/24".

## [0.7.0] - 2023-05-11

### Changed

- Upgrade minimum supported Rust version (MSRV) to 1.65.
- Upgrade rustls to 0.21.
- Upgrade quinn to 0.10.
- Remove `agent_id` and `host_id` fields from `AgentInfo` and `client_handshake`,
  as they are no longer passed to server manually. It's required that the server
  retrieve "{agent_id}@{host_id}" from Common Name (CN) of certificate used by the
  client.

## [0.6.1] - 2023-04-17

### Added

- `RequestCode::GetConfig` to get config toml file
- `RequestCode::SetConfig` to set config toml file
- More fields to `Configuration`
- `RequestCode::DeleteSamplingPolicy`
- `RequestCode::InternalNetworkList`

### Fixed

- Fix the trusted_domain_list update failure.

## [0.6.0] - 2022-11-28

### Added

- Add `RequestCode::SamplingPolicyList` for updating sampling policies.
- Add `RequestCode::ReloadFilterRule` for updating traffic filtering rules of piglet.

### Removed

- `ResourceUsage` and functions related with getting resource usage.

## [0.5.0] - 2022-11-07

### Changed

- Requires Rust 1.64 or later.
- Requires quinn 0.9.0 or later.
- `server_handshake` takes `&Connection` instead of `&mut IncomingBiStreams`.

## [0.4.1] - 2022-08-24

### Added

- Add `RequestCode::TorExitNodeList` for updating tor exit nodes.

## [0.4.0] - 2022-06-26

### Added

- When the received request code is not recognized, the response code is set to
  `RequestCode::Unknown`.
- `message::send_request` and `message::send_forward_request` can accept any
  type implementing `Into<u32>` as `code`.

### Changed

- `RequestCode` no longer implements `Serialize` and `Deserialize`; it should be
  converted to/from `u32` for serialization/deserialization.
- `message::recv_request_raw` returns the request code as `u32` instead of
  `RequestCode`.
- A client sending an unknown request code will receive an error message.

## [0.3.0] - 2022-06-24

### Added

- `message::server_handshake` receives a handshake message and sends a response.
- `AgentInfo` implements `Display`.
- `AgentInfo::key` to obtain the agent's key.

### Changed

- Requires Rust 1.57.0 or later.
- `message::handshake` is now `message::client_handshake` to differentiate
  itself from the server handshake.
- `HandshakeError::NewerProtocolRequired` has been replaced with
  `HandshakeError::IncompatibleProtocol`, which carries both the current version
  and the required version.
- `HandhsakeError::StreamError` has been replaced with
  `HandshakeError::ConnectionLost`.

## [0.2.7] - 2022-06-08

### Added

- `messages::handshake` and the types it uses: `AgentInfo` and `HandshakeError`.
  It sends a handshake request and processes the response.

## [0.2.6] - 2022-05-27

### Added

- `request::Handler` defines the interface for a request handler for an agent.
- `request::handle` handles requests for an agent.

## [0.2.5] - 2022-05-23

### Added

- `send_ok` and `send_err` to send a response.

## [0.2.4] - 2022-05-18

### Added

- `handle_resource_usage` to handle the `ResourceUsage` request.

## [0.2.3] - 2022-05-17

### Added

- `RequestCode` supports serialization with serde.
- `recv_request_raw` to receive a request without deserialization.
- `send_request` to send a request with serialization.
- `send_forward_request` to send a `RequestCode::Forward` request.

## [0.2.2] - 2022-05-16

### Added

- `frame::send_raw` to send raw bytes in a frame.

## [0.2.1] - 2022-05-12

### Added

- `RequestCode::ReloadConfig` to request agent to reload configuration
- `RequestCode::ResourceUsage` to request agent to collect resource usage stats

## [0.2.0] - 2022-05-11

### Added

- `RequestCode` that identifies the type of request.

## Changed

- `recv_frame`, `recv_raw_frame`, and `recv_raw_frame` are now under `frame`;
  use `frame::recv`, `frame::recv_raw`, and `frame::recv_raw_frame` instead.

## [0.1.1] - 2022-05-10

### Added

- `recv_raw_frame` to receive length-delimited frames without deserializing
  them.

## [0.1.0] - 2022-05-09

### Added

- `send_frame` and `recv_frame` to send and receive length-delimited frames.

[Unreleased]: https://github.com/petabi/oinq/compare/0.11.0...main
[0.11.0]: https://github.com/petabi/oinq/compare/0.10.0...0.11.0
[0.10.0]: https://github.com/petabi/oinq/compare/0.9.3...0.10.0
[0.9.3]: https://github.com/petabi/oinq/compare/0.9.2...0.9.3
[0.9.2]: https://github.com/petabi/oinq/compare/0.9.1...0.9.2
[0.9.1]: https://github.com/petabi/oinq/compare/0.9.0...0.9.1
[0.9.0]: https://github.com/petabi/oinq/compare/0.8.2...0.9.0
[0.8.2]: https://github.com/petabi/oinq/compare/0.8.1...0.8.2
[0.8.1]: https://github.com/petabi/oinq/compare/0.8.0...0.8.1
[0.8.0]: https://github.com/petabi/oinq/compare/0.7.1...0.8.0
[0.7.1]: https://github.com/petabi/oinq/compare/0.7.0...0.7.1
[0.7.0]: https://github.com/petabi/oinq/compare/0.6.1...0.7.0
[0.6.1]: https://github.com/petabi/oinq/compare/0.6.0...0.6.1
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

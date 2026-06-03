# Changelog

## [0.12.0](https://github.com/lumpinif/agents-router/compare/v0.11.0...v0.12.0) (2026-06-03)


### Features

* add agent controller runtime contract ([5613623](https://github.com/lumpinif/agents-router/commit/5613623a9bbba991e728306cb2abf9073f04b4b1))
* add agent integration continuation catalog ([a9deb39](https://github.com/lumpinif/agents-router/commit/a9deb39244413bc3c2fe9f8bcbdc634b1f46321c))
* add Codex App Server controller adapter ([2179e07](https://github.com/lumpinif/agents-router/commit/2179e07a02a6c04b208bf3d8a9f50b56de151590))
* add feishu lark app bot config contract ([156d0a7](https://github.com/lumpinif/agents-router/commit/156d0a7866c0d3283c1cd1cb53df66659cd65efb))
* add feishu lark app bot outbound receipt ([33e0127](https://github.com/lumpinif/agents-router/commit/33e01275c6b1739b48607fdb31bad2012b9f6181))
* add Feishu Lark App Bot setup ([5be2fac](https://github.com/lumpinif/agents-router/commit/5be2fac17dd667797e04e398071bfcc12ddb2551))
* add feishu lark hidden long connection contract ([6aed29b](https://github.com/lumpinif/agents-router/commit/6aed29b317928bcab58f12d28a0a432bcfb0d082))
* add feishu lark thread result replies ([9f96ca9](https://github.com/lumpinif/agents-router/commit/9f96ca937af4489a6a76e1f2d9cc380a1aea210b))
* add hidden Lark closed-loop contract ([270baa0](https://github.com/lumpinif/agents-router/commit/270baa0a91424130f76292f0b4480474b660324d))
* add Lark hidden long connection transport ([bbdb1ac](https://github.com/lumpinif/agents-router/commit/bbdb1ac8dc4d3dce08e17e668e5298674d6b1d01))
* add Lark project room new sessions ([eeb7858](https://github.com/lumpinif/agents-router/commit/eeb7858496fd6e8ca29a516d0682193eb39f6297))
* add minimal Lark room controls ([8f020b9](https://github.com/lumpinif/agents-router/commit/8f020b9bb2dd69be3583cf6adcb853018f3f8cb3))
* add provider inbound surface lookup ([8221f63](https://github.com/lumpinif/agents-router/commit/8221f636dc00b029d0e41c3bc5fb50f2f0ca3747))
* add response surface exposure gate ([2a36cc6](https://github.com/lumpinif/agents-router/commit/2a36cc61f42027265d7e9dce30904342f95cf6a8))
* add response surface ledger ([1642be0](https://github.com/lumpinif/agents-router/commit/1642be034e7fefa4df5600fe45323dd6b7044021))
* add response surface policy gate ([a77853a](https://github.com/lumpinif/agents-router/commit/a77853a9befd5c46a8aaa205615f7ff3f6b98ad5))
* add watch runtime owner lock ([96c6eaa](https://github.com/lumpinif/agents-router/commit/96c6eaa7f7bf82f67a21c347cda733fac18a5a4c))
* enable Lark App Bot replies from setup config ([8220a6d](https://github.com/lumpinif/agents-router/commit/8220a6d841ea184c57f01b84e430b82ec4cb6e40))
* guide Lark rooms and broken threads ([fc6f55c](https://github.com/lumpinif/agents-router/commit/fc6f55ce3e7848449a7a87338ab38fab7fa947a4))
* let Lark direct chats start Codex sessions ([378d7de](https://github.com/lumpinif/agents-router/commit/378d7dea7679d9976ea5c550d479a67b1c23ac6a))
* make Lark Personal Agent project rooms work ([7e6debb](https://github.com/lumpinif/agents-router/commit/7e6debbf0737066d5bac2a16b642988d39dcc889))
* make Lark project rooms work end to end ([00d685d](https://github.com/lumpinif/agents-router/commit/00d685d18b9062fd227797fa8d84b3709006ddfe))
* mark Codex Desktop continuation experimental available ([84febf7](https://github.com/lumpinif/agents-router/commit/84febf7827465bfeb458fabcf4a82bae891148b1))
* polish Lark Personal Agent control experience ([e4c50cc](https://github.com/lumpinif/agents-router/commit/e4c50ccb563d74b64cbf350dedc3a26d3b58de2e))
* stream Lark replies in cards ([6f7aaae](https://github.com/lumpinif/agents-router/commit/6f7aaae25dc9c2a9ec50b464a2e70d8de5f22575))
* verify feishu lark root lookup receipt ([f7e72a6](https://github.com/lumpinif/agents-router/commit/f7e72a604891bd21d66c9b971e23749010a158d2))
* wire hidden Lark closed loop ([5c6ff1f](https://github.com/lumpinif/agents-router/commit/5c6ff1f0a97f583775530aeb047d29dc8e17aee3))


### Bug Fixes

* align App Bot reply maturity semantics ([a004882](https://github.com/lumpinif/agents-router/commit/a004882abeada895451f4a8c37c2ef5117f2c9ea))
* clarify Codex unknown result feedback ([5441233](https://github.com/lumpinif/agents-router/commit/5441233324f2cc247fdb6c4e248baf60f2367a52))
* clarify Lark room control guidance ([6b4e068](https://github.com/lumpinif/agents-router/commit/6b4e06821c733470dd969bd467cec0602add2522))
* clarify Lark setup wording ([de460a7](https://github.com/lumpinif/agents-router/commit/de460a76b351d0a53977f1b24e1eeafa397605ad))
* collect App Bot secret directly in setup ([c4094cc](https://github.com/lumpinif/agents-router/commit/c4094cce1169e52d86ec62a11e442b58fd080f17))
* escape Lark card image markers ([7df9125](https://github.com/lumpinif/agents-router/commit/7df912565dab1afa78e995069c5c8ed906382e4e))
* fail closed on active Codex threads ([4cfb4dc](https://github.com/lumpinif/agents-router/commit/4cfb4dc21688862443ad39b7debe19055c729c1f))
* make Lark continuation recovery reliable ([d6a36e8](https://github.com/lumpinif/agents-router/commit/d6a36e8cd7f209c83bf581af8522fe96350f95df))
* make Lark room commands show real mentions ([88385ed](https://github.com/lumpinif/agents-router/commit/88385ed7ed4b714e0900b034531897ba8ddc257c))
* persist submitted Codex continuation boundary ([bfc0bf7](https://github.com/lumpinif/agents-router/commit/bfc0bf700e03d9372bf5449f9515ccc36e07e9e9))
* record submitted continuation outcomes ([244ebe9](https://github.com/lumpinif/agents-router/commit/244ebe995592074ee9aab9a6f469bd6be660ca25))
* remove response surface time windows ([94706e9](https://github.com/lumpinif/agents-router/commit/94706e9a6b89bd167d0269a4bd5b6268fc2eb365))
* require live ingress ping in status ([042b078](https://github.com/lumpinif/agents-router/commit/042b078062b9a317af97c4971eecc4f17241b870))
* resume unloaded Codex threads before continuation ([d5cd9d7](https://github.com/lumpinif/agents-router/commit/d5cd9d70a3ad3b399f454cf4e908bf18c4fa3116))
* skip Codex Desktop downtime backlog on restart ([b0db2de](https://github.com/lumpinif/agents-router/commit/b0db2ded19cb91dbb9f8b0e5fbfa8df42b7642da))
* skip retries for permanent Lark thread reply failures ([87d39f2](https://github.com/lumpinif/agents-router/commit/87d39f2f23824bb0e57437c0d732c2dc8aae43d9))
* stabilize macOS service binary identity ([bd55c42](https://github.com/lumpinif/agents-router/commit/bd55c42f57bf78032699ca35a3326594315a6822))
* start Lark streaming cards immediately ([fc16de2](https://github.com/lumpinif/agents-router/commit/fc16de295a0db070c07540f7e4afe47218676ff2))
* suppress response surface Codex completion cards ([448b03d](https://github.com/lumpinif/agents-router/commit/448b03d96b79edae2bc7cc2f4645211807d0aed9))
* tighten continuation duplicate safety ([5237296](https://github.com/lumpinif/agents-router/commit/523729685f1c2f63e94df22786c9a8629042809e))

## [0.11.0](https://github.com/lumpinif/agents-router/compare/v0.10.2...v0.11.0) (2026-05-20)


### Features

* make Slack the default setup provider ([9c86c4e](https://github.com/lumpinif/agents-router/commit/9c86c4e165e867e903e7c3e016f2934e941f1124))

## [0.10.2](https://github.com/lumpinif/agents-router/compare/v0.10.1...v0.10.2) (2026-05-17)


### Bug Fixes

* remove deprecated hooks usage ([de8d91d](https://github.com/lumpinif/agents-router/commit/de8d91d9807ece369d110c4762ec3c3a843c1ce5))

## [0.10.1](https://github.com/lumpinif/agents-router/compare/v0.10.0...v0.10.1) (2026-05-14)


### Bug Fixes

* show session id after notification time ([3d53e1b](https://github.com/lumpinif/agents-router/commit/3d53e1b6ec166bd0a555ced8dc1e9362fbaaa1dd))

## [0.10.0](https://github.com/lumpinif/agents-router/compare/v0.9.1...v0.10.0) (2026-05-14)


### Features

* add delivery safety guard ([237d3c8](https://github.com/lumpinif/agents-router/commit/237d3c8a027d6ec03f8669854531ebf6f00849fa))
* reconcile hook-based source integrations ([6b61221](https://github.com/lumpinif/agents-router/commit/6b612214308c2a87f29c1535dd8b2f676d545af8))
* support explicit emit durations ([fc41f45](https://github.com/lumpinif/agents-router/commit/fc41f454e2eba1c1956d11d4eb98b6515a32ef19))


### Bug Fixes

* align setup provider ids with provider types ([098f466](https://github.com/lumpinif/agents-router/commit/098f4661e8f5ada6f61442f4ae07e4e0be0e1b1a))
* harden codex index and provider env urls ([d10f29d](https://github.com/lumpinif/agents-router/commit/d10f29df3cabf3068ac4f94c554fbb32e8c7109f))
* harden routing and service reliability ([3b8a6a4](https://github.com/lumpinif/agents-router/commit/3b8a6a44065087811d1b336e7238cc4817dc4cf8))
* ignore desktop-origin Codex CLI hooks ([00e3b96](https://github.com/lumpinif/agents-router/commit/00e3b96ae1862d1c0d30942d85cd190bc97c68e0))
* prevent Codex Desktop replay after provider failure ([377721c](https://github.com/lumpinif/agents-router/commit/377721ce8a5d6ff5f85ffb3171af82bf012183d4))

## [0.9.1](https://github.com/lumpinif/agents-router/compare/v0.9.0...v0.9.1) (2026-05-13)


### Bug Fixes

* preserve cargo-installed legacy binary during migration ([b17d90c](https://github.com/lumpinif/agents-router/commit/b17d90cee15d894afe1607fedc83ec75d927fec0))

## [0.9.0](https://github.com/lumpinif/agents-router/compare/v0.8.1...v0.9.0) (2026-05-13)


### Features

* rename project to Agents Router ([71207c0](https://github.com/lumpinif/agents-router/commit/71207c06f92a9edf9162e97059cdf309f6ba187f))

## 0.8.1

This release improves setup, agent hooks, and upgrades.

- Easier setup with notification preferences.
- Better support for structured CLI agent hooks.
- Optional filters for long-running tasks and selected projects.
- Config changes reload automatically when valid.
- Installers now restart the running service after upgrade.
- Weixin is now named WeChat.
- Fixed Codex Desktop model detection.

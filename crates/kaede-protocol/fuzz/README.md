kaede-protocol fuzz
====================

CBOR 解码契约：ClientMessage::decode / ServerMessage::decode 对任意字节输入永不 panic。

本机（Windows）验证
-------------------
cd crates/kaede-protocol/fuzz
cargo +nightly fuzz build

能 build 即通过契约编译检查。运行 cargo fuzz run 需要 ASan runtime（Windows 本地不便，见 CI）。

CI（Linux，推荐）
-----------------
把以下步骤加入 GitHub Actions：
1. uses: dtolnay/rust-toolchain@nightly
2. run: cargo install cargo-fuzz --locked
3. run: cd crates/kaede-protocol/fuzz && cargo +nightly fuzz build
4. run: cd crates/kaede-protocol/fuzz && cargo +nightly fuzz run decode_client -- -max_total_time=60
5. run: cd crates/kaede-protocol/fuzz && cargo +nightly fuzz run decode_server -- -max_total_time=60

为什么需要 fuzz
---------------
服务端每帧调用 ClientMessage::decode。恶意客户端可发任意字节；任何 panic 都是远程 DoS。
属性测试（tests/decode_fuzz.rs 的 quickcheck）覆盖有限样本；cargo-fuzz 用覆盖率引导持续搜索边界。

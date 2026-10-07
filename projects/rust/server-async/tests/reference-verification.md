# 终端与参考程序手动测试清单

下面的项目由你在终端操作，核对结果后勾选。初始空框表示待手动验证，不表示此前发现了功能错误。Rust 自动化测试继续覆盖字段类型、大小边界、令牌过期、超时与并发一致性。

## 启动程序

所有命令在 `projects/rust/server-async/` 目录执行。每组使用同一个地址，每次只启动一个服务端；切换组合前先按 Ctrl-C 停止当前服务端。重启会清空账号和文本，每组都需重新注册。

| 组合 | 终端 A：服务端 | 终端 B：客户端 |
| --- | --- | --- |
| 自己的客户端与服务端 | `cargo run --locked -- --address 127.0.0.1:7878` | `cargo run --locked --manifest-path ../client-sync/Cargo.toml -- --url http://127.0.0.1:7878` |
| 自己的客户端 → 参考服务端 | `./rm-projects-rust-linux-x86_64-reference-v0.2.1/rm-server-async --address 127.0.0.1:7878` | `cargo run --locked --manifest-path ../client-sync/Cargo.toml -- --url http://127.0.0.1:7878` |
| 参考客户端 → 自己的服务端 | `cargo run --locked -- --address 127.0.0.1:7878` | `./rm-projects-rust-linux-x86_64-reference-v0.2.1/rm-client-sync --url http://127.0.0.1:7878` |

第二个用户使用终端 C，再启动同一种客户端，连接同一个服务端。

### 多行输入区别

自己的客户端：单独一行 `.` 结束并保留末尾换行，`.end` 结束并去掉最后一个换行。参考客户端：单独一行 `.` 结束，默认不附加末尾换行；需要末尾换行时，在结束前增加一行空行；不要用 `.end` 结束。两者的正文以点开头时，都要多输入一个点。

发送正文 `你好\nRM`，自己的客户端输入：

```text
echo
你好
RM
.end
```

参考客户端输入同样的两行正文，最后使用 `.`。

自己的客户端通常在一行显示状态与 JSON；参考客户端显示 `HTTP <状态>`，成功的 `get`/`echo` 直接显示正文。检查状态和内容，不要求排版一致。

## 1. 账号与正文

在终端 B 使用用户名 `alice`、密码 `password1`，按顺序操作：

- [ ] `ping`：200，数据为 `pong`。
- [ ] `register`：201，返回用户名；密码输入不显示。重复注册返回 409。
- [ ] `login` 使用密码 `wrongpassword`：401；随后仍能输入命令。
- [ ] 正确密码登录：200，包含非空 `token` 和正整数 `expires_in`；`list` 返回空列表。
- [ ] `echo` 输入 Unicode、空行、空格、多行正文，原样返回；再试空正文。
- [ ] `put` 名称 `note`、正文 `你好\nRM`：200；`get note` 返回相同内容。
- [ ] 再次 `put note` 改为 `changed`，读取确认更新。
- [ ] 保存名称 `z`、`A`、`a`，其中 `A` 正文为空；读取 `A` 返回空文本。
- [ ] `list` 返回 `A`、`a`、`note`、`z`，名称区分大小写且升序。
- [ ] `delete note`：200；重复删除与读取都返回 404；列表移除 `note`。
- [ ] 删除其他文本后，列表为空。

这里的 `get note` 等表示先输入 `get`，再按名称提示输入 `note`，不是同一行命令。

## 2. 用户隔离、令牌与注销

终端 B 保持 alice 登录；终端 C 注册并登录 bob，密码也可用 `password1`：

- [ ] 两个用户分别保存同名 `note`，正文为 `alice text` 和 `bob text`；读取只得到自己的内容。
- [ ] alice 覆盖 `note`，bob 内容不变；alice 另存 `alice-only`，bob 列表不出现该名称。
- [ ] alice 删除 `note` 后读取返回 404，bob 仍能读取自己的 `note`。
- [ ] 终端 C `logout` 退出 bob，再登录 alice；终端 B 的 `list` 返回 401，提示重新登录。
- [ ] 终端 B 重新登录 alice 后恢复访问；`logout` 返回 200，之后 `list` 返回 401。
- [ ] 终端 C 重新登录 bob；终端 B 重新登录 alice，再 `delete-user`：200。alice 的 `list` 与原账号登录均返回 401。
- [ ] 重新注册并登录 alice，列表为空；bob 的 `note` 仍存在。

## 3. 错误、退出与重启

- [ ] 客户端输入非法名称 `bad/name`：提示错误；随后 `ping` 成功。
- [ ] 输入 `q` 返回 shell；重开客户端，在等待命令时按 Ctrl-C，也能退出。
- [ ] 服务端按 Ctrl-C 退出；客户端再次 `ping` 报连接错误，仍能输入命令和 `q`。
- [ ] 重启服务端，`ping` 恢复；旧令牌无法访问、旧账号无法登录；重新注册同名账号成功，列表为空。
- [ ] 服务端启动参数 `--address invalid`、`--token-ttl-seconds 0`，客户端参数 `--unknown-option`，均明确报错并退出。

### 请求失败后的恢复

服务端运行时，在另一终端发送非法 JSON：

```bash
curl -i -H 'Content-Type: application/json' --data-binary '{x}' http://127.0.0.1:7878/echo
curl -i http://127.0.0.1:7878/ping
```

- [ ] 第一条返回 400，第二条返回 200；已有文本内容不变。

模拟上传中途断开（curl 因 1 秒超时而报错是预期）：

```bash
curl -i --max-time 1 --limit-rate 1 -H 'Content-Type: application/json' --data-binary '{"text":"interrupted body"}' http://127.0.0.1:7878/echo
curl -i http://127.0.0.1:7878/ping
```

- [ ] 中断请求后，`ping` 返回 200，其他用户的文本仍可读取。

## 手动结果记录

每种组合分别完成上述步骤。记录日期、环境、版本及未通过项目的请求、状态与实际输出。

| 组合 | 手动验证日期 | 结果或未通过项目 |
| --- | --- | --- |
| 自己的客户端与服务端 | 待填写 | 待手动验证 |
| 自己的客户端 → 参考服务端 | 待填写 | 待手动验证 |
| 参考客户端 → 自己的服务端 | 待填写 | 待手动验证 |

## 发行版与历史验证记录

- 来源：[GitHub Release reference-v0.2.1](https://github.com/trident-rm/DGP-Orientation-2026fall-projects/releases/tag/reference-v0.2.1)。
- 平台：`x86_64-unknown-linux-gnu`；材料提交：`288db1c9cf92fabf9bce299a70b580394ec302b7`。与本地协议只有一处文档链接差异，接口规则一致。
- SHA-256：`b507ad959862dd0cb79788ed3ebe3c53f6e5494d9716b8cc3cabe6d56d241701`。再次校验可执行 `sha256sum -c rm-projects-rust-linux-x86_64-reference-v0.2.1.sha256`。
- 2026-10-07，助手用自动化终端脚本完成双向对接，两组各 4 项通过，覆盖账号、文本、隔离、错误恢复、退出与重启；本地实现提交为 `9516a13856dda3f05888501d857c23026721d5d8`，另有 `http.rs` 注释修改。
- 按当前选择，终端自动化脚本已移除。历史通过结果不代表你已完成本清单的手动验证。

下载的程序已具备执行权限。复制后若权限丢失，可用 `chmod u+x ./rm-projects-rust-linux-x86_64-reference-v0.2.1/rm-server-async` 恢复对应文件的执行权限。

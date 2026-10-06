# Rust 异步服务端

完成内容见[公共异步服务端任务](../../../common/tasks.md#异步服务端)。开发环境、检查命令和启动命令见 [Rust 路线](../README.md)，必须在本目录执行。

## 代码结构

| 路径 | 职责 |
| --- | --- |
| `src/lib.rs` | 业务规则、用户状态和文本状态 |
| `src/main.rs` | 命令行参数、`Service` 构造和启动入口 |
| `src/http.rs` | Rocket 请求处理、应用组装和阻塞工作调度 |
| `tests/service.rs` | 业务逻辑测试 |
| `tests/http.rs` | HTTP 层测试 |

## 开发说明

`lib.rs` 继续承载同步业务逻辑和状态锁；`http.rs` 负责异步请求处理，并用 `rocket::tokio::task::spawn_blocking` 把密码计算等可能阻塞的操作移出运行时工作线程。实现时注意区分“可以直接在异步任务中完成的轻量逻辑”和“必须放到阻塞线程的同步业务”，并保持共享状态保护正确。

HTTP 验证见[异步服务端](../../../common/tasks.md#异步服务端)。

## 令牌有效期

`--token-ttl-seconds` 接受正整数，默认 300 秒。例如：

```bash
cargo run --locked -- --address 127.0.0.1:7878 --token-ttl-seconds 60
```

登录成功响应包含 `token` 和 `expires_in`（配置的有效秒数）。
有效期从登录成功起计算，访问文本不会续期；到期后所有受保护接口返回 401，
需要重新登录。退出登录或注销账号会立即撤销令牌。

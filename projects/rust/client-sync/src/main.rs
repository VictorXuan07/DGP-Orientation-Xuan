use clap::Parser; //处理命令行参数
use reqwest::blocking::Client; //用于发送HTTP请求
use serde_json::{Value, json}; //用于处理JSON数据
use std::io::{self, Write}; //用于处理输入输出，self表示把io本身也引入，后面可以不用再加std；
                            //write是一个trait，提供了flush方法，用于刷新缓冲区，暂时还没有完全理解
use std::time::Duration; //时间，这里大概是用来处理网络超时的问题

#[derive(Parser)] //#[derive(...)]是attribute(属性)，表示请编译器/宏系统帮这个类型自动生成某些能力。
                    //Parser是clap提供的一个宏，用于自动生成命令行参数解析的代码
//宏(macro)：像代码生成器，我写的代码交给宏，宏展开生成新的代码，新的代码再交给编译器编译。宏的作用是减少重复代码，提高开发效率。
//它的样子是：xxx!
//而clap的Parser宏是一个derive宏，它的样子是：#[derive(Parser)]，它的作用是为结构体生成命令行参数解析的代码。

// 结构体struct和class类似，但rust会把数据和方法分开，struct只负责存储数据，方法需要单独实现（impl）
struct Args  //定义一个结构体Args，用于存储命令行参数，和Java的那个args差不多
{ 
    #[arg(long, default_value = "http://127.0.0.1:7878")] //告诉clap下面这个 url 字段应该如何对应命令行参数。
                                                          //long表示这个参数是一个长选项，将输入的url转化为--url。
//input()的作用：在终端显示提示文字 → 等待用户输入一行 → 去掉末尾换行符 → 把用户输入的字符串返回。
fn input(prompt: &str) -> io::Result<String> {
    print!("{prompt}"); //print!宏和println!宏类似，区别是print!宏不换行。prompt可能会被放到缓冲区不能保证输出。
    io::stdout().flush()?; //flush()方法会把缓冲区的内容立即输出到终端，?表示如果flush()返回错误，就直接返回这个错误。
    let mut line = String::new(); //let:声明一个可变变量；mut：可变变量；String::new():创建一个空的字符串，
                                  //不能使用&str，因为&str是不可变的字符串切片，不能存储用户输入的内容。
    if io::stdin().read_line(&mut line)? == 0 {  //把 line 的可变引用交给 read_line()，允许它修改这个 String。
                                                 //read_line()的返回值是读取的字节数，如果为0，说明用户输入了EOF（End Of File），也就是Ctrl+D。
        return Err(io::ErrorKind::UnexpectedEof.into());//into()方法把io::ErrorKind::UnexpectedEof转换为io::Error类型，方便返回错误。
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())//删除末尾的换行符和回车符，to_owned()方法把&str转换为String类型，方便返回。
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut token = String::new();
    loop {
        let command = match input(
            "ping / register / login / logout / list / echo / delete-user / put / get / delete / q > ",
        ) {
            Ok(command) => command,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        };
        let mut body = Value::Null;
        let (method, path) = match command.as_str() {
            "q" => break,
            "ping" => ("GET", "/ping"),
            "list" => ("GET", "/texts"),
            "logout" => ("DELETE", "/sessions/current"),
            "register" | "login" => {
                body = json!({"username": input("username: ")?, "password": rpassword::prompt_password("password: ")?});
                (
                    "POST",
                    if command == "register" {
                        "/users"
                    } else {
                        "/sessions"
                    },
                )
            }
            "echo" | "delete-user" | "put" | "get" | "delete" => {
                println!("This task is not implemented in the starting code yet.");
                continue;
            }
            _ => {
                println!("Unknown command.");
                continue;
            }
        };
        let result = rm_client_sync::exchange(
            &client,
            &args.url,
            method.parse().unwrap(),
            path,
            &token,
            if body.is_null() { None } else { Some(&body) },
        );
        match result {
            Ok((status, value)) => {
                println!("{status} {value}");
                if command == "login"
                    && status == 200
                    && let Some(next) = value["data"]["token"].as_str()
                {
                    token = next.into();
                }
                if status == 401 {
                    println!("Please log in again.");
                }
                if status == 401 || (command == "logout" && status == 200) {
                    token.clear();
                }
            }
            Err(error) => eprintln!("Request failed: {error}"),
        }
    }
    Ok(())
}

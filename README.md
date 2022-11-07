# rss_to_twitter_bot_lambda
Rust implementation of a twitter bot to tweet from multiple twitter accounts and use multiple RSS/Atom Feeds as source. AWS Lambda Version

Can be used for RSS or ATOM feeds

Bott is currently set up to tweet in a format like
title
link
eg.

This Week in Rust 467
https://this-week-in-rust.org/blog/2022/11/02/this-week-in-rust-467/



Requirements:
- rust aws-lambda-runtime: https://github.com/awslabs/aws-lambda-rust-runtime   
- Use Homebrew on [MacOS](https://brew.sh/):
```bash
brew tap cargo-lambda/cargo-lambda
```
```bash
brew install cargo-lambda
```

to build for Amazon Linux 2 Lambda run
```bash
cargo lambda build --release --arm64
```

Upload to AWS Lambda
1. zip the Bootstrap file in target/lambda/{your_project_name}/
2. Upload the Bootstrap.zip to your Lambda Function. [Here are some ways to do so](https://github.com/awslabs/aws-lambda-rust-runtime/blob/main/README.md#2-deploying-the-binary-to-aws-lambda)
3. Set a trigger for your Lambda function 
    e.g. An EventBrigde scheduled trigger (every 15 min)


To use
- Add RSS/Atom Feed URLs inside the main.rs
- Register your Twitter Account on the [Twitter Developer Portal](https://developer.twitter.com/en/portal/dashboard)
 the you can get your keys and secrets required to access the Twitter API.
 Place the keys and secrets in an Account e.g. in common/TwitterAccount1


known bugs:
- Some feeds hide their content behind a cloudflare 5 sec protection. Currently the Bot hosted in Lambda can not handle such feeds.

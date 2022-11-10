use aws_lambda_events::event::cloudwatch_events::CloudWatchEvent;use lambda_runtime::{run, service_fn, Error as LambdaError, LambdaEvent};
use atom_syndication::{Entry, Feed as AtomFeed};
use chrono::{DateTime, Utc};
use egg_mode::direct::DraftMessage;
use egg_mode::tweet::DraftTweet;
use rss::{Channel, Item};
use std::error::Error;
use std::{time::{UNIX_EPOCH, Duration}};

const TYPE_RSS: &str = "RSS";
const TYPE_ATOM: &str = "ATOM";
const CRON_TIME: u64 = 3600; // 1h in seconds
const MY_TWITTER_ID: u64 = 123456789; // This Twitter User gets a DM in case of error
const ALLOW_TWEETS: bool = true; // Global tweet / not tweet switch
const ALLOW_ERROR_DM: bool = false; // Get a twitter DM from you bot account to your personal account when something fails. Your Twitter must be set in MY_TWITTER_ID

#[derive(Clone)]
pub struct Feed {
    name: String,
    url: String,
    feed_type: &'static str,
    filter: String,
}

#[derive(Clone)]
pub struct TwitterAccount {
    pub name: String,
    rss_feeds: Vec<Feed>,
    token: egg_mode::Token,
    latitude: f64,
    longitude: f64,
    display_location: bool,
    timestamp_lambda: u64,
}

async fn get_url_content(url: &str) -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::builder().user_agent("Mozilla/5.0 (X11; Ubuntu; Linux i686; rv:24.0) Gecko/20100101 Firefox/24.0 PACKAGE_NAME admin@<COMPANY>.com").build()?;
    let result = client.get(url).timeout(Duration::from_secs(10)).send().await;
    if result.is_err() {
        eprintln!("Error in fn 'get_url_content'  for Feed {}",url);
    } 
    match result.unwrap().text().await {
        Ok(c) => Ok(c),
        Err(e) => Err(Box::new(e)),
        
    }
}

async fn send_tweets(
    account: TwitterAccount,
    feed: Feed,
    title: String,
    link: &str,
) -> Result<(), Box<dyn Error>> {
    // filter
    let mut send_tweet = true;
    let mut filter = feed.filter.clone();

    if filter != "" {
        // Filter is set
        if filter.contains("INCLUDE:") {
            send_tweet = false; // block tweet until include matches
            filter = filter.replace("INCLUDE:", "");
            let split = filter.split(',');
            for s in split {
                if title.contains(s) {
                    send_tweet = true; //match found: allow tweet
                }
            }
        } else if filter.contains("EXCLUDE:") {
            filter = filter.replace("EXCLUDE:", "");
            let split = filter.split(',');
            for s in split {
                if title.contains(s) {
                    send_tweet = false; // exclude match found: block tweet
                }
            }
        }
    }

    if send_tweet == true {
        let text = title.clone() + "\n" + link.clone();
        let draft = DraftTweet::new(text.clone()).coordinates(
            account.latitude,
            account.longitude,
            account.display_location,
        );
        println!("Tweet: {}",text);
        if ALLOW_TWEETS {
            let _send_tweet = draft.send(&account.token).await;
        }
    }

    Ok(())
}

async fn filter_tweets_rss(
    account: TwitterAccount,
    items: Vec<Item>,
    feed: Feed,
) -> Result<(), Box<dyn Error>> {
    let timestamp = account.timestamp_lambda.clone();

    for item in items {
        let item_time_string = &item.pub_date().unwrap();
        let item_timestamp:u64;
        if item_time_string.contains("Z") { // need to filter which Date format is used. Default is rfc2822
            item_timestamp = DateTime::parse_from_rfc3339(item_time_string).unwrap().timestamp() as u64;
        } else {
            item_timestamp = DateTime::parse_from_rfc2822(item_time_string).unwrap().timestamp() as u64;
        }

        if item_timestamp > timestamp {
            let _ = send_tweets(
                account.clone(),
                feed.clone(),
                item.title().unwrap().to_owned(),
                item.link().unwrap_or(""),
            ).await;
        }
    }
    Ok(())
}

async fn filter_tweets_atom(
    account: TwitterAccount,
    entries: Vec<Entry>,
    feed: Feed,
) -> Result<(), Box<dyn Error>> {
    let timestamp = account.timestamp_lambda.clone();

    for entry in entries {
        let entry_timestamp: u64;
        if entry.published().is_none() {
            // No publish date. Use Update Date
            entry_timestamp = entry.updated().timestamp() as u64;
        } else {
            entry_timestamp = entry.published().unwrap().timestamp() as u64;
        }
        
        if entry_timestamp > timestamp {
            let _ = send_tweets(
                account.clone(),
                feed.clone(),
                entry.title().value.to_string(),
                entry.links().first().unwrap().href(),
            ).await;
        }
    }
    Ok(())
}

async fn post_tweets_for_account(account: TwitterAccount) -> Result<(), Box<dyn Error>> {
    let feeds = account.rss_feeds.clone();
    for feed in feeds.iter() {
        println!("current Feed: {}", feed.name.clone());

        let content = get_url_content(feed.url.as_str()).await.unwrap();

        if feed.feed_type == "RSS" {
            let result = Channel::read_from(&content.as_bytes()[..]);
            match result {
                Ok(n) => _ = filter_tweets_rss(account.clone(), Vec::from(n.items()), feed.clone()).await,
                Err(e) => _ = send_error_twitter_dm(account.clone(), feed.clone(), Box::new(e)).await,
            }
        } else if feed.feed_type == "ATOM" {
            let result = content.parse::<AtomFeed>();
            match result {
                Ok(n) => _ = filter_tweets_atom(account.clone(), Vec::from(n.entries()), feed.clone()).await,
                Err(e) => _ = send_error_twitter_dm(account.clone(), feed.clone(), Box::new(e)).await,
            }
        }
    }

    Ok(())
}

async fn send_error_twitter_dm(
    account: TwitterAccount,
    feed: Feed,
    error: Box<dyn Error>,
) -> Result<(), Box<dyn Error>> {
    let d = UNIX_EPOCH + Duration::from_secs(account.timestamp_lambda);
    let datetime = DateTime::<Utc>::from(d).format("%Y-%m-%d %H:%M:%S").to_string() + " UTC";
    let text = "Feed: ".to_string() + &feed.name + " at " + &datetime + "\n" + &error.to_string();
        
    eprintln!("{}", &text);
    if ALLOW_ERROR_DM {
        let message = DraftMessage::new(text, MY_TWITTER_ID);
        let _ = message.send(&account.token).await.unwrap();
    }
    Ok(())
}

/// This is the main body for the Lambda function.
async fn function_handler(event: LambdaEvent<CloudWatchEvent>) -> Result<(), LambdaError> {

    // Offset the timestamp with the cronjob time to get your last intervall of feeds
    let timestamp = event.payload.time.timestamp() as u64 - CRON_TIME; 
    println!("Region: {}",event.payload.region.unwrap());
    println!("Time: {}",event.payload.time);

    let my_account_1: TwitterAccount = TwitterAccount {
        name: String::from("My Automated Twitter Account 1"),
        rss_feeds: vec![Feed {
            name: String::from("RSS Feed"),
            url: String::from("https://www.example.xml"),
            feed_type: TYPE_RSS,
            filter: String::from(""),
        },Feed {
            name: String::from("ATOM Feed"),
            url: String::from("https://www.example.rss"),
            feed_type: TYPE_ATOM,
            filter: String::from(""),
        }],
        token: egg_mode::Token::Access {
            consumer: egg_mode::KeyPair::new(
                include_str!("common/TwitterAccount1/consumer_key").trim(),
                include_str!("common/TwitterAccount1/consumer_secret").trim(),
            ),
            access: egg_mode::KeyPair::new(
                include_str!("common/TwitterAccount1/access_key").trim(),
                include_str!("common/TwitterAccount1/access_secret").trim(),
            ),
        },
        latitude: 51.509865, // Location London, UK
        longitude: -0.118092,
        display_location: true, // toggle if you want to display location in tweet. Location must be enabled in twitter settings
        timestamp_lambda: timestamp.clone(),
    };

    // Be aware that there are 2 structurally differend feed types: RSS and ATOM 
    // To specity a tweet use the const TYPE_ATOM or TYPE_RSS in the feed_type field 
    let my_account_2: TwitterAccount = TwitterAccount {
        name: String::from("My Automated Twitter Account 2"),
        rss_feeds: vec![
            Feed{name: String::from("RSS 1"), url: String::from("https://www.example.rss"), feed_type: TYPE_RSS, filter: String::from("")},
            // Include only feed Items/entries that include "Rust" in title
            Feed{name: String::from("RSS 2"), url: String::from("https://www.example2.rss"), feed_type: TYPE_RSS, filter: String::from("INCLUDE:Rust")},
            Feed{name: String::from("RSS 3"), url: String::from("https://www.example3.rss"), feed_type: TYPE_RSS, filter: String::from("")},
            // Exclude feeds that contain the title "iPhone"
            Feed{name: String::from("Apple"), url: String::from("https://www.apple.com/newsroom/rss-feed.rss"), feed_type: TYPE_ATOM, filter: String::from("EXCLUDE:iPhone")},
        ],
        token: egg_mode::Token::Access {
            consumer: egg_mode::KeyPair::new(
                include_str!("common/TwitterAccount2/consumer_key").trim(),
                include_str!("common/TwitterAccount2/consumer_secret").trim(),
            ),
            access: egg_mode::KeyPair::new(
                include_str!("common/TwitterAccount2/access_key").trim(),
                include_str!("common/TwitterAccount2/access_secret").trim(),
            ),
        },
        latitude: 51.509865,
        longitude: -0.118092,
        display_location: false,
        timestamp_lambda: timestamp.clone(),
    };

    // starts the RSS fetch and twitter posting
    let _result = post_tweets_for_account(my_account_1).await;
    let _result = post_tweets_for_account(my_account_2).await;

    Ok(())
}


#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    // disable printing the name of the module in every log line.
    .with_target(false)
    // disabling time is handy because CloudWatch will add the ingestion time.
    .without_time()
    .init();

    let _ = run(service_fn(function_handler)).await;
}

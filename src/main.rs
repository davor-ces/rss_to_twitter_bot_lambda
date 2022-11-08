use aws_lambda_events::event::cloudwatch_events::CloudWatchEvent;use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use atom_syndication::{Entry, Feed as AtomFeed};
use chrono::{DateTime};
use egg_mode::direct::DraftMessage;
use egg_mode::tweet::DraftTweet;
use rss::{Channel, Item};
use std::error::Error as OtherError;

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

async fn get_rss_items(url: &str) -> Result<Vec<rss::Item>, Box<dyn OtherError>> {
    let content = reqwest::get(url).await?.text().await?;
    let channel = Channel::read_from(&content.as_bytes()[..]);
    if channel.is_err() {
        eprintln!("Error for rss channel in 'get_rss_items' for url\n{}",url);
    }
    Ok(Vec::from(channel?.items()))
}

async fn get_atom_entries(url: &str) -> Result<Vec<Entry>, Box<dyn OtherError>> {
    let content = reqwest::get(url).await?.text().await?;
    let feed = content.parse::<AtomFeed>();
    if feed.is_err() {
        eprintln!("Error for atom feed in 'get_atom_entries' for url\n{}",url);
    }
    Ok(Vec::from(feed?.entries()))
}

async fn filter_and_send_tweets(
    account: TwitterAccount,
    feed: Feed,
    title: String,
    link: &str,
) -> Result<(), Box<dyn OtherError>> {
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

async fn send_tweets_rss(
    account: TwitterAccount,
    items: Vec<Item>,
    feed: Feed,
) -> Result<(), Box<dyn OtherError>> {
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
            let _ = filter_and_send_tweets(
                account.clone(),
                feed.clone(),
                item.title().unwrap().to_string(),
                item.link().unwrap(),
            ).await;
        }
    }
    Ok(())
}

async fn post_tweets_atom(
    account: TwitterAccount,
    entries: Vec<Entry>,
    feed: Feed,
) -> Result<(), Box<dyn OtherError>> {
    let entry_timestamp: u64;
        if entry.published().is_none() {
            // No publish date. Use Update Date
            entry_timestamp = entry.updated().timestamp() as u64;
        } else {
            entry_timestamp = entry.published().unwrap().timestamp() as u64;
        }

    for entry in entries {
        let entry_timestamp = entry.updated().timestamp() as u64;

        if entry_timestamp > timestamp {
            let _ = filter_and_send_tweets(
                account.clone(),
                feed.clone(),
                entry.title().value.to_string(),
                entry.links().first().unwrap().href(),
            ).await;
        }
    }
    Ok(())
}

async fn post_tweets_for_account(account: TwitterAccount) -> Result<(), Box<dyn OtherError>> {
    let feeds = account.rss_feeds.clone();
    for feed in feeds.iter() {
        println!("current Feed: {}", feed.name.clone());

        if feed.feed_type == "RSS" {
            let result = get_rss_items(feed.url.as_str()).await;
            match result {
                Ok(n) => _ = send_tweets_rss(account.clone(), n, feed.clone()).await,
                Err(e) => _ = send_error_twitter_dm(account.clone(), feed.clone(), e).await,
            }
        } else if feed.feed_type == "ATOM" {
            let result = get_atom_entries(feed.url.as_str()).await;
            match result {
                Ok(n) => _ = post_tweets_atom(account.clone(), n, feed.clone()).await,
                Err(e) => _ = send_error_twitter_dm(account.clone(), feed.clone(), e).await,
            }
        }
    }

    Ok(())
}

async fn send_error_twitter_dm(
    account: TwitterAccount,
    feed: Feed,
    error: Box<dyn OtherError>,
) -> Result<(), Box<dyn OtherError>> {
    
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
async fn function_handler(event: LambdaEvent<CloudWatchEvent>) -> Result<(), Error> {

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
            Feed{name: String::from("RSS 1"), url: String::from("https://matt-rickard.com/rss"), feed_type: TYPE_RSS, filter: String::from("")},
            // Include only feed Items/entries that include "Rust" in title
            Feed{name: String::from("RSS 2"), url: String::from("https://www.pragcap.com/feed/"), feed_type: TYPE_RSS, filter: String::from("INCLUDE:Rust")},
            Feed{name: String::from("RSS 3"), url: String::from("https://unfashionable.substack.com/feed/"), feed_type: TYPE_RSS, filter: String::from("")},
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
        latitude: 47.14151,
        longitude: 9.52154,
        display_location: false,
        timestamp_lambda: timestamp.clone(),
    };

    // starts the RSS fetch and twitter posting
    let _result = post_tweets_for_account(my_account_1).await;
    let _result = post_tweets_for_account(my_account_2).await;

    Ok(())
}


#[tokio::main]
async fn main()  -> Result<(), Error> {
    tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    // disable printing the name of the module in every log line.
    .with_target(false)
    // disabling time is handy because CloudWatch will add the ingestion time.
    .without_time()
    .init();

    run(service_fn(function_handler)).await
}

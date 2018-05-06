extern crate chrono;
extern crate icalendar;
extern crate kuchiki;
extern crate regex;
extern crate reqwest;
extern crate selectors;
extern crate url;

#[macro_use]
extern crate clap;

fn main() {
    let matches = clap::App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .setting(clap::AppSettings::SubcommandRequiredElseHelp)
        .subcommand(
            clap::SubCommand::with_name("generate")
                .about("Generate iCalendar file for friends scout schedule")
                .arg(clap::Arg::with_name("URL").takes_value(true).required(true)),
        )
        .get_matches();

    match matches.subcommand() {
        ("generate", Some(sub)) => subcommand_generate(sub),
        _ => unreachable!(),
    }
}

fn subcommand_generate(matches: &clap::ArgMatches) {
    let base_uri = url::Url::parse(matches.value_of("URL").unwrap()).expect("Failed to parse URL");

    let mut headers = reqwest::header::Headers::new();
    headers.set(reqwest::header::UserAgent::new(format!(
        "{}/{}",
        crate_name!(),
        crate_version!()
    )));
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();

    let mut resp = client
        .get(base_uri.clone())
        .send()
        .expect("Failed to send GET request");
    let document = {
        use kuchiki::traits::TendrilSink;
        kuchiki::parse_html()
            .from_utf8()
            .read_from(&mut resp)
            .expect("Failed to read HTML")
    };
    let event_title_node = document
        .select("#title img")
        .unwrap()
        .next()
        .expect("Failed to find title");
    let event_title_attrs = event_title_node.attributes.borrow();
    let event_title = event_title_attrs
        .get("alt")
        .expect("title node doesn't have alt attribute");
    for area in document.select("#m_shop area[href]").unwrap() {
        let attrs = area.attributes.borrow();
        let href = attrs
            .get("href")
            .expect("area element doesn't have href attribute");
        let title = attrs
            .get("title")
            .expect("area element doesn't have title element");
        let shop_uri = base_uri.join(href).unwrap();
        write_calendar(&client, event_title, title, shop_uri);
    }
}

fn write_calendar(client: &reqwest::Client, event_title: &str, title: &str, shop_uri: url::Url) {
    let mut resp = client.get(shop_uri.clone()).send().unwrap_or_else(|_| {
        panic!("Failed to send GET request to {}", shop_uri);
    });
    let document = {
        use kuchiki::traits::TendrilSink;
        kuchiki::parse_html()
            .from_utf8()
            .read_from(&mut resp)
            .expect("Failed to read HTML")
    };

    let re = regex::Regex::new(r"(?s)(\d+)年(\d+)月(\d+)日.+/(\d+):(\d+)～").unwrap();
    let mut calendar = icalendar::Calendar::new();

    for table in document
        .select(".shoplist_resultlist[cellpadding]")
        .unwrap()
    {
        let rows: Vec<_> = table.as_node().select("tr").unwrap().collect();
        if rows.len() != 3 {
            panic!(
                "{} shop table has unexpected rows: {}",
                shop_uri,
                rows.len()
            );
        }
        let shop_name = rows[0]
            .as_node()
            .select(".shopname")
            .unwrap()
            .next()
            .expect("Failed to find shop name")
            .text_contents();
        let shop_name = shop_name.trim();
        let address = rows[1]
            .as_node()
            .select(".list-adtext-detitext")
            .unwrap()
            .next()
            .expect("Failed to find address")
            .text_contents();
        let address = address.trim();

        let mut start_cap = false;
        for child in rows[2]
            .as_node()
            .select(".list-adtext-detitext > div > strong,hr")
            .unwrap()
        {
            use selectors::Element;
            if child.get_local_name() == "strong" {
                if !start_cap {
                    let desc = child.text_contents();
                    let cap = re.captures(&desc).unwrap_or_else(|| {
                        panic!(
                            "{} {} has unrecognizable description: {}",
                            shop_uri, shop_name, desc
                        );
                    });
                    let start_time = chrono::DateTime::parse_from_rfc3339(&format!(
                        "{}-{}-{}T{:02}:{}:00+09:00",
                        &cap[1], &cap[2], &cap[3], &cap[4], &cap[5]
                    )).unwrap();
                    let end_time = start_time + chrono::Duration::hours(1);
                    let event = {
                        use icalendar::Component;

                        icalendar::Event::new()
                            .summary(event_title)
                            .description(&format!("{}\n{}", shop_name, shop_uri))
                            .location(address)
                            .starts(start_time)
                            .ends(end_time)
                            .done()
                    };
                    calendar.push(event);
                    start_cap = true;
                }
            } else {
                start_cap = false;
            }
        }
    }
    if calendar.is_empty() {
        eprintln!("{} has no events", title);
        return;
    }

    let mut file = std::fs::File::create(&format!("{}.ics", title)).expect("Failed to open file");
    {
        use std::io::Write;
        file.write_all(calendar.to_string().as_bytes())
            .expect("Failed to write iCal");
    }
}

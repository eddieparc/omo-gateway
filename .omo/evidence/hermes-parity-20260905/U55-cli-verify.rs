use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("fixture path");
    let values = dotenvy::from_path_iter(&path)
        .expect("fixture opens")
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()
        .expect("dotenv parses");
    assert_eq!(
        values.get("DISCORD_BOT_TOKEN").map(String::as_str),
        Some("u55-fixture-token")
    );
    assert_eq!(
        values.get("OPENAI_API_KEY").map(String::as_str),
        Some("a # b $HOME")
    );
    assert_eq!(
        values.get("KEEP").map(String::as_str),
        Some("literal $HOME # value")
    );
    assert!(std::fs::read_to_string(path)
        .expect("read fixture")
        .starts_with(
            "# retained synthetic fixture\nexport KEEP='literal $HOME # value' # preserve\n"
        ));
    println!("U55_CLI_VERIFIED token=true secret=true raw_prefix=true");
}

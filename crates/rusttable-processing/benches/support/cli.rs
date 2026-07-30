use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Check(Option<PathBuf>),
    List(Option<PathBuf>),
}

pub fn parse(arguments: &[String]) -> Result<Command, String> {
    let start = match (
        arguments.first().map(String::as_str),
        arguments.get(1).map(String::as_str),
    ) {
        (Some("--bench"), _) => 0,
        (_, Some("--bench")) => 1,
        _ => {
            return Err("missing leading --bench sentinel".to_owned());
        }
    };
    let arguments = &arguments[start..];
    if arguments.first().map(String::as_str) != Some("--bench") {
        return Err("missing leading --bench sentinel".to_owned());
    }
    if arguments
        .iter()
        .skip(1)
        .any(|argument| argument == "--bench")
    {
        return Err("duplicate --bench sentinel".to_owned());
    }
    let mut command = "check";
    let mut check_seen = false;
    let mut list_seen = false;
    let mut config = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--check" if !check_seen && !list_seen => {
                check_seen = true;
            }
            "--list" if !list_seen && !check_seen => {
                list_seen = true;
                command = "list";
            }
            "--config" if config.is_none() && index + 1 < arguments.len() => {
                config = Some(PathBuf::from(&arguments[index + 1]));
                index += 1;
            }
            _ => return Err("unknown, repeated, or incomplete benchmark argument".to_owned()),
        }
        index += 1;
    }
    Ok(if command == "list" {
        Command::List(config)
    } else {
        Command::Check(config)
    })
}

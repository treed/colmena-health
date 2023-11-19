use std::{collections::HashMap, str::FromStr};

use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, take_till1, take_while},
    character::complete::alphanumeric1,
    combinator::{map_res, recognize},
    multi::{many0, separated_list1},
    sequence::{delimited, tuple},
    Finish, IResult,
};

fn label_name(i: &str) -> IResult<&str, &str> {
    // This is just the RFC1123 standard for hostnames
    // I figured that it's better to start more restrictive and loosen it later if needed
    recognize(tuple((
        alphanumeric1,
        many0(tuple((take_while(|c: char| c == '-'), alphanumeric1))),
    )))(i)
}

fn label_value(i: &str) -> IResult<&str, Box<dyn TermMatcher>> {
    alt((regex_value, list_value))(i)
}

fn list_value(i: &str) -> IResult<&str, Box<dyn TermMatcher>> {
    let (input, list) = separated_list1(tag(","), value_item)(i)?;

    Ok((input, Box::new(ListMatcher::new(list))))
}

fn value_item(i: &str) -> IResult<&str, &str> {
    take_till1(|c| c == ',' || c == ' ' || c == '\t')(i)
}

fn regex_value(i: &str) -> IResult<&str, Box<dyn TermMatcher>> {
    let (input, regex) = map_res(delimited(tag("/"), is_not("/"), tag("/")), regex::Regex::new)(i)?;

    Ok((input, Box::new(RegexMatcher::new(regex))))
}

pub fn term(i: &str) -> IResult<&str, Term> {
    let (input, (name, _, matcher)) = tuple((label_name, tag(":"), label_value))(i)?;

    Ok((
        input,
        Term {
            name: name.to_string(),
            matcher,
        },
    ))
}

pub struct Term {
    name: String,
    matcher: Box<dyn TermMatcher>,
}

impl Term {
    pub fn matches(&self, labels: &HashMap<String, String>) -> bool {
        match labels.get(&self.name) {
            None => false,
            Some(value) => self.matcher.matches(value),
        }
    }
}

impl FromStr for Term {
    type Err = nom::error::Error<String>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match term(s).finish() {
            Ok((_remaining, name)) => Ok(name),
            Err(nom::error::Error { input, code }) => Err(nom::error::Error {
                input: input.to_string(),
                code,
            }),
        }
    }
}

trait TermMatcher {
    fn matches(&self, label_value: &str) -> bool;
}

struct ListMatcher {
    list: Vec<String>,
}

impl ListMatcher {
    fn new(list: Vec<&str>) -> ListMatcher {
        ListMatcher {
            list: list.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl TermMatcher for ListMatcher {
    fn matches(&self, label_value: &str) -> bool {
        self.list.iter().any(|item| item == label_value)
    }
}

struct RegexMatcher {
    regex: regex::Regex,
}

impl RegexMatcher {
    fn new(regex: regex::Regex) -> RegexMatcher {
        RegexMatcher { regex }
    }
}

impl TermMatcher for RegexMatcher {
    fn matches(&self, label_value: &str) -> bool {
        self.regex.is_match(label_value)
    }
}

#[cfg(test)]
mod tests {
    use nom::combinator::all_consuming;

    use super::*;

    fn get_matcher<T>(parsed: Result<(&str, T), nom::Err<nom::error::Error<&str>>>) -> T {
        let (rest, matcher) = parsed.unwrap();

        assert_eq!(rest, "");
        matcher
    }

    #[test]
    fn test_label_names() {
        assert!(label_name("").is_err());
        assert!(label_name("foo").is_ok());
        assert!(label_name("foo3").is_ok());
        assert!(label_name("3foo3").is_ok());
        assert!(all_consuming(label_name)("foo-").is_err());
        assert!(label_name("foo-bar").is_ok());
        assert!(label_name("foo--bar").is_ok());
        assert!(label_name("-foo").is_err());
    }

    #[test]
    fn test_regex_matcher() {
        let matcher = get_matcher(regex_value("/^rack203-.*/"));

        assert!(matcher.matches("rack203-cl14"));
        assert!(!matcher.matches("arack20-cl140"));
    }

    #[test]
    fn test_list_matcher() {
        let matcher = get_matcher(list_value("foo,bar,baz"));

        assert!(matcher.matches("foo"));
        assert!(!matcher.matches("fooo"));
        assert!(matcher.matches("baz"));
    }

    #[test]
    fn test_terms() {
        let labels = HashMap::from([
            ("hostname".to_owned(), "test-host".to_owned()),
            ("role".to_owned(), "web".to_owned()),
            ("rack".to_owned(), "23".to_owned()),
        ]);

        assert!(get_matcher(term("hostname:test-host")).matches(&labels));
        assert!(get_matcher(term("hostname:test-host,test-host2")).matches(&labels));
        assert!(!get_matcher(term("hostname:other-host,some-other-host")).matches(&labels));
        assert!(get_matcher(term("hostname:/test-/")).matches(&labels));
        assert!(!get_matcher(term("hostname:/test-$/")).matches(&labels));
    }
}

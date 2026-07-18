#!/usr/bin/env bash
set -euo pipefail

if (( $# != 1 )); then
  printf 'usage: %s REPOSITORY_ROOT\n' "$0" >&2
  exit 2
fi

repository_root="$1"
manifest="$repository_root/package.json"
if [[ ! -d "$repository_root" ]]; then
  printf 'bun-pin: repository root is not a directory: %s\n' "$repository_root" >&2
  exit 1
fi
if [[ ! -f "$manifest" ]]; then
  printf 'bun-pin: package.json is missing: %s\n' "$manifest" >&2
  exit 1
fi

awk '
function fail(message) {
  if (error == "") error = message
}

function skip_whitespace(    character) {
  while (position <= source_length) {
    character = substr(source, position, 1)
    if (character == " " || character == "\t" || character == "\r" || character == "\n") {
      position++
    } else {
      return
    }
  }
}

function parse_string(    character, escaped, unicode_index, value) {
  if (substr(source, position, 1) != "\"") {
    fail("malformed JSON: expected string")
    return ""
  }
  position++
  value = ""
  while (position <= source_length) {
    character = substr(source, position, 1)
    if (character == "\"") {
      position++
      return value
    }
    if (character ~ /[[:cntrl:]]/) {
      fail("malformed JSON: unescaped control character in string")
      position++
      return value
    }
    if (character == "\\") {
      position++
      if (position > source_length) {
        fail("malformed JSON: unterminated escape")
        return value
      }
      escaped = substr(source, position, 1)
      if (escaped == "u") {
        if (position + 4 > source_length) {
          fail("malformed JSON: incomplete unicode escape")
          return value
        }
        for (unicode_index = 1; unicode_index <= 4; unicode_index++) {
          if (substr(source, position + unicode_index, 1) !~ /^[0-9A-Fa-f]$/) {
            fail("malformed JSON: invalid unicode escape")
            return value
          }
        }
        value = value "\\u" substr(source, position + 1, 4)
        position += 5
      } else if (escaped == "\"" || escaped == "\\" || escaped == "/" || escaped == "b" || escaped == "f" || escaped == "n" || escaped == "r" || escaped == "t") {
        value = value "\\" escaped
        position++
      } else {
        fail("malformed JSON: invalid escape")
        return value
      }
    } else {
      value = value character
      position++
    }
  }
  fail("malformed JSON: unterminated string")
  return value
}

function parse_number(    remainder, matched) {
  remainder = substr(source, position)
  matched = match(remainder, /^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?/)
  if (matched == 0) {
    fail("malformed JSON: invalid value")
    return
  }
  position += RLENGTH
}

function parse_literal(literal) {
  if (substr(source, position, length(literal)) != literal) {
    fail("malformed JSON: invalid value")
    return
  }
  position += length(literal)
}

function parse_value(depth,    character) {
  if (error != "") return
  skip_whitespace()
  character = substr(source, position, 1)
  if (character == "{") {
    parse_object(depth)
  } else if (character == "[") {
    parse_array(depth)
  } else if (character == "\"") {
    parse_string()
  } else if (character == "t") {
    parse_literal("true")
  } else if (character == "f") {
    parse_literal("false")
  } else if (character == "n") {
    parse_literal("null")
  } else if (character == "-" || character ~ /^[0-9]$/) {
    parse_number()
  } else {
    fail("malformed JSON: invalid value")
  }
}

function parse_array(depth,    character, first) {
  position++
  skip_whitespace()
  character = substr(source, position, 1)
  if (character == "]") {
    position++
    return
  }
  first = 1
  while (position <= source_length && error == "") {
    if (!first) {
      skip_whitespace()
      if (substr(source, position, 1) != ",") {
        fail("malformed JSON: expected comma in array")
        return
      }
      position++
    }
    parse_value(depth + 1)
    skip_whitespace()
    character = substr(source, position, 1)
    if (character == "]") {
      position++
      return
    }
    first = 0
  }
  fail("malformed JSON: unterminated array")
}

function parse_object(depth,    character, first, key, value) {
  position++
  skip_whitespace()
  character = substr(source, position, 1)
  if (character == "}") {
    position++
    return
  }
  first = 1
  while (position <= source_length && error == "") {
    if (!first) {
      skip_whitespace()
      if (substr(source, position, 1) != ",") {
        fail("malformed JSON: expected comma in object")
        return
      }
      position++
      skip_whitespace()
    }
    key = parse_string()
    skip_whitespace()
    if (substr(source, position, 1) != ":") {
      fail("malformed JSON: expected colon after object key")
      return
    }
    position++
    skip_whitespace()
    if (depth == 0 && key == "packageManager") {
      package_manager_count++
      if (substr(source, position, 1) != "\"") {
        fail("packageManager must be a JSON string")
        return
      }
      value = parse_string()
      if (package_manager_count == 1) package_manager = value
    } else {
      parse_value(depth + 1)
    }
    skip_whitespace()
    character = substr(source, position, 1)
    if (character == "}") {
      position++
      return
    }
    first = 0
  }
  fail("malformed JSON: unterminated object")
}

BEGIN {
  source = ""
  while ((getline line) > 0) source = source line "\n"
  source_length = length(source)
  position = 1
  skip_whitespace()
  if (substr(source, position, 1) != "{") {
    fail("package.json root must be a JSON object")
  } else {
    parse_object(0)
  }
  skip_whitespace()
  if (error == "" && position <= source_length) fail("malformed JSON: trailing data")
  if (error == "" && package_manager_count == 0) fail("packageManager is missing")
  if (error == "" && package_manager_count > 1) fail("packageManager is duplicated")
  if (error == "" && package_manager !~ /^bun@[0-9]+\.[0-9]+\.[0-9]+$/) fail("packageManager must be exactly bun@<numeric major.minor.patch>")
}

END {
  if (error != "") {
    print "bun-pin: " error > "/dev/stderr"
    exit 1
  }
  printf "bun-version=%s\n", substr(package_manager, 5)
}
' "$manifest"

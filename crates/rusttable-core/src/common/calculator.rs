//! Mathematical expression solver ported from Darktable's
//! `src/common/calculator.c` and `src/common/calculator.h`.

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Operator(Operator),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Plus,
    Increment,
    Minus,
    Decrement,
    Multiply,
    Division,
    Modulo,
    Power,
    Ratio,
    LeftRound,
    RightRound,
}

struct Scanner<'a> {
    formula: &'a str,
    position: usize,
    x: f64,
    invalid_number: bool,
}

impl<'a> Scanner<'a> {
    const fn new(formula: &'a str, x: f64) -> Self {
        Self {
            formula,
            position: 0,
            x,
            invalid_number: false,
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let bytes = self.formula.as_bytes();
        while self.position < bytes.len() {
            let byte = bytes[self.position];
            let operator = match byte {
                b'+' if bytes.get(self.position + 1) == Some(&b'+') => {
                    self.position += 2;
                    Operator::Increment
                }
                b'+' => {
                    self.position += 1;
                    Operator::Plus
                }
                b'-' if bytes.get(self.position + 1) == Some(&b'-') => {
                    self.position += 2;
                    Operator::Decrement
                }
                b'-' => {
                    self.position += 1;
                    Operator::Minus
                }
                b'*' => {
                    self.position += 1;
                    Operator::Multiply
                }
                b'/' => {
                    self.position += 1;
                    Operator::Division
                }
                b'%' => {
                    self.position += 1;
                    Operator::Modulo
                }
                b'^' => {
                    self.position += 1;
                    Operator::Power
                }
                b':' => {
                    self.position += 1;
                    Operator::Ratio
                }
                b'(' => {
                    self.position += 1;
                    Operator::LeftRound
                }
                b')' => {
                    self.position += 1;
                    Operator::RightRound
                }
                b'x' | b'X' => {
                    self.position += 1;
                    return Some(Token::Number(self.x));
                }
                b'0'..=b'9' | b'.' => return Some(Token::Number(self.read_number())),
                _ => {
                    self.position += 1;
                    continue;
                }
            };
            return Some(Token::Operator(operator));
        }
        None
    }

    fn read_number(&mut self) -> f64 {
        let start = self.position;
        if self.formula.as_bytes().get(start) == Some(&b'0')
            && matches!(self.formula.as_bytes().get(start + 1), Some(b'x' | b'X'))
            && let Some((value, end)) = parse_hexadecimal(&self.formula[start..])
        {
            self.position = start + end;
            return value;
        }

        let end = decimal_prefix_end(&self.formula[start..]);
        if end == 0 {
            self.position += 1;
            self.invalid_number = true;
            return f64::NAN;
        }
        self.position += end;
        self.formula[start..start + end]
            .parse::<f64>()
            .unwrap_or(f64::NAN)
    }
}

fn decimal_prefix_end(formula: &str) -> usize {
    let bytes = formula.as_bytes();
    let mut position = 0;
    let mut digits = 0;
    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
        position += 1;
        digits += 1;
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0;
    }

    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        let exponent_start = position;
        position += 1;
        if matches!(bytes.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        let digit_start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == digit_start {
            position = exponent_start;
        }
    }
    position
}

#[derive(Debug, Default)]
struct BinaryWindow {
    bits: u64,
    stored_bits: u32,
    tail_nonzero: bool,
}

impl BinaryWindow {
    fn push_digit(&mut self, digit: u8, width: u32) {
        for shift in (0..width).rev() {
            let bit = u64::from((digit >> shift) & 1);
            if self.stored_bits < u64::BITS {
                self.bits = (self.bits << 1) | bit;
                self.stored_bits += 1;
            } else if bit != 0 {
                self.tail_nonzero = true;
            }
        }
    }

    fn leading_bits(&self, count: u32) -> u64 {
        if count == 0 {
            0
        } else if self.stored_bits >= count {
            self.bits >> (self.stored_bits - count)
        } else {
            self.bits << (count - self.stored_bits)
        }
    }

    fn bit(&self, index: u32) -> bool {
        index < self.stored_bits && self.bits & (1_u64 << (self.stored_bits - index - 1)) != 0
    }

    fn has_bits_after(&self, index: u32) -> bool {
        let remaining = self.stored_bits.saturating_sub(index.saturating_add(1));
        self.tail_nonzero || (remaining != 0 && self.bits & ((1_u64 << remaining) - 1) != 0)
    }

    fn rounded(&self, retained_bits: u32) -> u64 {
        let retained = self.leading_bits(retained_bits);
        let guard = self.bit(retained_bits);
        if guard && (self.has_bits_after(retained_bits) || retained & 1 != 0) {
            retained + 1
        } else {
            retained
        }
    }
}

fn record_hexadecimal_digit(
    digit: u8,
    index: usize,
    first_nonzero_index: &mut Option<usize>,
    first_width: &mut u32,
    window: &mut BinaryWindow,
) {
    if first_nonzero_index.is_none() {
        if digit == 0 {
            return;
        }
        *first_nonzero_index = Some(index);
        *first_width = u8::BITS - digit.leading_zeros();
        window.push_digit(digit, *first_width);
    } else {
        window.push_digit(digit, 4);
    }
}

fn parse_hexadecimal_exponent(bytes: &[u8], position: &mut usize) -> i128 {
    if !matches!(bytes.get(*position), Some(b'p' | b'P')) {
        return 0;
    }

    let marker = *position;
    *position += 1;
    let negative = if bytes.get(*position) == Some(&b'-') {
        *position += 1;
        true
    } else {
        if bytes.get(*position) == Some(&b'+') {
            *position += 1;
        }
        false
    };
    let digit_start = *position;
    let input_length = i128::try_from(bytes.len()).unwrap_or(i128::MAX);
    let magnitude_cap = input_length.saturating_mul(4).saturating_add(4_096);
    let mut magnitude = 0_i128;
    while let Some(byte) = bytes.get(*position)
        && byte.is_ascii_digit()
    {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(i128::from(*byte - b'0'))
            .min(magnitude_cap);
        *position += 1;
    }
    if *position == digit_start {
        *position = marker;
        0
    } else if negative {
        -magnitude
    } else {
        magnitude
    }
}

fn hexadecimal_value(window: &BinaryWindow, mut leading_exponent: i128) -> f64 {
    if leading_exponent > 1_023 {
        return f64::INFINITY;
    }
    if leading_exponent < -1_075 {
        return 0.0;
    }
    if leading_exponent < -1_022 {
        let retained_bits = u32::try_from(leading_exponent + 1_075)
            .expect("subnormal hexadecimal precision is between zero and 52 bits");
        return f64::from_bits(window.rounded(retained_bits));
    }

    let mut significand = window.rounded(53);
    if significand == 1_u64 << 53 {
        significand >>= 1;
        leading_exponent += 1;
    }
    if leading_exponent > 1_023 {
        return f64::INFINITY;
    }

    let exponent = u64::try_from(leading_exponent + 1_023)
        .expect("normal hexadecimal exponent is non-negative");
    let fraction_mask = (1_u64 << 52) - 1;
    f64::from_bits((exponent << 52) | (significand & fraction_mask))
}

fn parse_hexadecimal(formula: &str) -> Option<(f64, usize)> {
    let bytes = formula.as_bytes();
    let mut position = 2;
    let mut digit_index = 0_usize;
    let mut integer_digits = 0_usize;
    let mut first_nonzero_index = None;
    let mut first_width = 0_u32;
    let mut window = BinaryWindow::default();

    while let Some(digit) = bytes
        .get(position)
        .and_then(|byte| hexadecimal_digit(*byte))
    {
        record_hexadecimal_digit(
            digit,
            digit_index,
            &mut first_nonzero_index,
            &mut first_width,
            &mut window,
        );
        position += 1;
        digit_index += 1;
        integer_digits += 1;
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        while let Some(digit) = bytes
            .get(position)
            .and_then(|byte| hexadecimal_digit(*byte))
        {
            record_hexadecimal_digit(
                digit,
                digit_index,
                &mut first_nonzero_index,
                &mut first_width,
                &mut window,
            );
            position += 1;
            digit_index += 1;
        }
    }
    if digit_index == 0 {
        return None;
    }

    let exponent = parse_hexadecimal_exponent(bytes, &mut position);
    let Some(first_nonzero_index) = first_nonzero_index else {
        return Some((0.0, position));
    };
    let integer_digits = i128::try_from(integer_digits).unwrap_or(i128::MAX);
    let first_nonzero_index = i128::try_from(first_nonzero_index).unwrap_or(i128::MAX);
    let digit_offset = integer_digits
        .saturating_sub(first_nonzero_index)
        .saturating_sub(1);
    let leading_exponent = exponent
        .saturating_add(digit_offset.saturating_mul(4))
        .saturating_add(i128::from(first_width - 1));
    Some((hexadecimal_value(&window, leading_exponent), position))
}

const fn hexadecimal_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct Parser<'a> {
    scanner: Scanner<'a>,
    token: Option<Token>,
}

impl<'a> Parser<'a> {
    fn new(formula: &'a str, x: f64) -> Self {
        let mut scanner = Scanner::new(formula, x);
        let token = scanner.next_token();
        Self { scanner, token }
    }

    fn advance(&mut self) {
        self.token = self.scanner.next_token();
    }

    fn parse_expression(&mut self) -> f64 {
        self.parse_additive_expression()
    }

    fn parse_additive_expression(&mut self) -> f64 {
        if self.token.is_none() {
            return f64::NAN;
        }
        let mut left = self.parse_multiplicative_expression();
        loop {
            let Some(Token::Operator(operator @ (Operator::Plus | Operator::Minus))) = self.token
            else {
                return left;
            };
            self.advance();
            let right = self.parse_multiplicative_expression();
            if operator == Operator::Plus {
                left += right;
            } else {
                left -= right;
            }
        }
    }

    fn parse_multiplicative_expression(&mut self) -> f64 {
        if self.token.is_none() {
            return f64::NAN;
        }
        let mut left = self.parse_power_expression();
        loop {
            let Some(Token::Operator(
                operator @ (Operator::Multiply
                | Operator::Division
                | Operator::Modulo
                | Operator::Ratio),
            )) = self.token
            else {
                return left;
            };
            self.advance();
            let right = self.parse_power_expression();
            left = match operator {
                Operator::Multiply => left * right,
                Operator::Division => left / right,
                Operator::Modulo => left % right,
                Operator::Ratio => c_ratio(left, right),
                _ => unreachable!("matched multiplicative operator"),
            };
        }
    }

    fn parse_power_expression(&mut self) -> f64 {
        if self.token.is_none() {
            return f64::NAN;
        }
        let mut left = self.parse_unary_expression();
        while self.token == Some(Token::Operator(Operator::Power)) {
            self.advance();
            let right = self.parse_unary_expression();
            left = left.powf(right);
        }
        left
    }

    fn parse_unary_expression(&mut self) -> f64 {
        match self.token {
            Some(Token::Operator(Operator::Minus)) => {
                self.advance();
                -self.parse_unary_expression()
            }
            Some(Token::Operator(Operator::Plus)) => {
                self.advance();
                self.parse_unary_expression()
            }
            Some(_) => self.parse_primary_expression(),
            None => f64::NAN,
        }
    }

    fn parse_primary_expression(&mut self) -> f64 {
        match self.token {
            Some(Token::Number(number)) => {
                self.advance();
                number
            }
            Some(Token::Operator(Operator::LeftRound)) => {
                self.advance();
                let result = self.parse_expression();
                if self.token != Some(Token::Operator(Operator::RightRound)) {
                    return f64::NAN;
                }
                self.advance();
                result
            }
            _ => f64::NAN,
        }
    }
}

fn c_ratio(left: f64, right: f64) -> f64 {
    let maximum = if left > right { left } else { right };
    let minimum = if left < right { left } else { right };
    maximum / minimum
}

/// Solves one Darktable calculator formula with `x` as its only variable.
///
/// A missing or empty formula returns `NaN`, matching
/// `dt_calculator_solve`.
#[must_use]
pub fn solve(x: f64, formula: Option<&str>) -> f64 {
    let Some(formula) = formula else {
        return f64::NAN;
    };
    let formula = formula.split('\0').next().unwrap_or_default();
    if formula.is_empty() {
        return f64::NAN;
    }
    let normalized = formula.replace(',', ".");
    let mut parser = Parser::new(&normalized, x);
    match parser.token {
        Some(Token::Operator(Operator::Increment)) => x + 1.0,
        Some(Token::Operator(Operator::Decrement)) => x - 1.0,
        _ => {
            let result = parser.parse_expression();
            if parser.token.is_some() || parser.scanner.invalid_number {
                f64::NAN
            } else {
                result
            }
        }
    }
}

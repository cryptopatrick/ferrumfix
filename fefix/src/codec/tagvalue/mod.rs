//! FIX tag-value encoding support.
//!
//! This is the original encoding used for FIX messages and also the encoding
//! currently used by the FIX session layer.

use crate::backend::value as val;
use crate::backend::{slr, Backend, FixFieldValue, Version};
use crate::buffering::Buffer;
use crate::codec::{Encoding, StreamingDecoder};
use crate::dbglog;
use crate::dictionary::Dictionary;
use crate::DataType;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::rc::Rc;
use std::str;

mod agnostic;
mod utils;

pub use agnostic::{AgnosticMessage, CodecAgnostic};

/// Easy-to-use [`Encoding`] that accomodates for most use cases.
#[derive(Debug)]
pub struct Codec<T, Z>
where
    Z: Config,
{
    dict: Dictionary,
    message: T,
    agnostic_codec: CodecAgnostic<Z>,
    config: Z,
}

impl<T, Z> Codec<T, Z>
where
    T: Default,
    Z: Config,
{
    /// Builds a new [`Codec`] encoding device with a FIX 4.4 dictionary.
    pub fn new(config: Z) -> Self {
        Self::with_dict(Dictionary::from_version(Version::Fix44), config)
    }

    /// Creates a new codec for the tag-value format. `dict` is used to parse messages.
    pub fn with_dict(dict: Dictionary, config: Z) -> Self {
        let mut agnostic_codec = CodecAgnostic::<Z>::default();
        *agnostic_codec.config_mut() = config.clone();
        Self {
            dict,
            agnostic_codec,
            message: T::default(),
            config,
        }
    }

    /// Returns an immutable reference to the [`Config`] used by `self`.
    pub fn config(&self) -> &Z {
        &self.config
    }

    /// Returns a mutable reference to the [`Config`] used by `self`.
    pub fn config_mut(&mut self) -> &mut Z {
        &mut self.config
    }
}

impl<T, Z> Encoding<T> for Codec<T, Z>
where
    T: Backend<FixFieldValue>,
    Z: Config,
{
    type DecodeError = DecodeError;
    type EncodeError = EncodeError;

    fn decode(&mut self, data: &[u8]) -> Result<&T, Self::DecodeError> {
        // Take care of `BeginString`, `BodyLength` and `CheckSum`.
        let agnostic_message = self.agnostic_codec.decode(data)?;
        let field_begin_string = agnostic_message.field_begin_string();
        // Empty the message.
        self.message.clear();
        let mut fields =
            &mut FieldIter::new(agnostic_message.body(), self.config.clone(), &self.dict);
        // Deserialize `MsgType(35)`.
        let msg_type = {
            let mut f = fields.next().ok_or(Self::DecodeError::Syntax)??;
            if f.tag() != 35 {
                dbglog!("Expected MsgType (35), got ({}) instead.", f.tag());
                return Err(Self::DecodeError::Syntax);
            }
            f.take_value()
        };
        self.message
            .insert(8, FixFieldValue::string(field_begin_string).unwrap())
            .unwrap();
        self.message.insert(35, msg_type).unwrap();
        // Iterate over all the other fields and store them to the message.
        for field_result in &mut fields {
            let mut field = field_result?;
            dbglog!("Finished parsing field <{}>.", field.tag());
            self.message
                .insert(field.tag(), field.take_value())
                .unwrap();
        }
        Ok(&self.message)
    }

    fn encode<B>(&mut self, buffer: &mut B, message: &T) -> Result<usize, Self::EncodeError>
    where
        B: Buffer,
    {
        let body_writer = |buffer: &mut B| {
            let start_i = buffer.as_slice().len();
            message
                .for_each::<(), _>(|tag, value| {
                    if tag != 8 {
                        encode_field((tag as u16).into(), value, buffer, self.config.separator());
                    }
                    Ok(())
                })
                .unwrap();
            buffer.as_slice().len() - start_i
        };
        let field_begin_string = message.field(8).unwrap().as_str().unwrap().as_bytes();
        utils::encode(&self.config, field_begin_string, body_writer, buffer)
    }
}

fn encode_field(tag: val::TagNum, value: &FixFieldValue, write: &mut impl Buffer, separator: u8) {
    write.extend_from_slice(tag.to_string().as_bytes());
    write.extend_from_slice(&[b'=']);
    match &value {
        FixFieldValue::String(s) => write.extend_from_slice(s.as_bytes()),
        FixFieldValue::Group(_) => panic!("Can't encode a group!"),
        FixFieldValue::Atom(field) => write.extend_from_slice(field.to_string().as_bytes()),
    };
    write.extend_from_slice(&[separator]);
}

/// A (de)serializer for the classic FIX tag-value encoding.
///
/// The FIX tag-value encoding is designed to be both human-readable and easy for
/// machines to parse.
///
/// Please reach out to the FIX official documentation[^1][^2] for more information.
///
/// [^1]: [FIX TagValue Encoding: Online reference.](https://www.fixtrading.org/standards/tagvalue-online)
///
/// [^2]: [FIX TagValue Encoding: PDF.](https://www.fixtrading.org/standards/tagvalue/)
#[derive(Debug)]
pub struct CodecBuffered<T, Z>
where
    Z: Config,
{
    buffer: Vec<u8>,
    buffer_relevant_len: usize,
    buffer_additional_len: usize,
    codec: Codec<T, Z>,
}

impl<T, Z> CodecBuffered<T, Z>
where
    T: Default,
    Z: Config,
{
    /// Builds a new `Codec` encoding device with a FIX 4.4 dictionary.
    pub fn new(config: Z) -> Self {
        Self::with_dict(Dictionary::from_version(Version::Fix44), config)
    }

    /// Creates a new codec for the tag-value format. `dict` is used to parse messages.
    pub fn with_dict(dict: Dictionary, config: Z) -> Self {
        Self {
            buffer: Vec::new(),
            buffer_relevant_len: 0,
            buffer_additional_len: 0,
            codec: Codec::with_dict(dict.clone(), config.clone()),
        }
    }
}

impl<T, Z> Encoding<T> for CodecBuffered<T, Z>
where
    T: Backend<FixFieldValue> + Default,
    Z: Config,
{
    type DecodeError = DecodeError;
    type EncodeError = EncodeError;

    fn decode(&mut self, data: &[u8]) -> Result<&T, Self::DecodeError> {
        self.codec.decode(data)
    }

    fn encode<B>(
        &mut self,
        buffer: &mut B,
        message: &T,
    ) -> std::result::Result<usize, Self::EncodeError>
    where
        B: Buffer,
    {
        self.codec.encode(buffer, message)
    }
}

impl<T, Z> StreamingDecoder<T> for CodecBuffered<T, Z>
where
    T: Backend<FixFieldValue> + Default,
    Z: Config,
{
    type Error = DecodeError;

    fn supply_buffer(&mut self) -> (&mut usize, &mut [u8]) {
        // 512 bytes at a time. Not optimal, but a reasonable default.
        let len = 512;
        self.buffer.resize(self.buffer_relevant_len + len, 0);
        (
            &mut self.buffer_additional_len,
            &mut self.buffer[self.buffer_relevant_len..self.buffer_relevant_len + len],
        )
    }

    fn attempt_decoding(&mut self) -> Result<Option<&T>, Self::Error> {
        self.buffer_relevant_len += self.buffer_additional_len;
        assert!(self.buffer_relevant_len <= self.buffer.len());
        if self.buffer_relevant_len < 10 {
            Ok(None)
        } else if &self.buffer[self.buffer_relevant_len - 7..self.buffer_relevant_len - 3] == b"10="
        {
            self.codec
                .decode(&self.buffer[..self.buffer_relevant_len])
                .map(|message| Some(message))
        } else {
            Ok(None)
        }
    }

    fn get(&self) -> &T {
        &self.codec.message
    }
}

/// This trait describes dynamic tag lookup logic.
///
/// In this context, "tag lookup"
/// means to search in the dictionary the data type associated with a specific
/// tag number. This may seem trivial at best, but it can actually be quite
/// convoluted and require internal state (thus it is "dynamic" tag lookup). In
/// particular, several fields affect the internal state of a
/// [`TagLookup`](TagLookup):
///
///  - `ApplVerID <1128>`
///  - `ApplExtID <1156>`
///  - `CstmApplVerID <1129>`
///  - `DefaultApplVerID <1137>`
///  - `DefaultApplExtID <1407>`
///  - `DefaultCstmApplVerID <1408>`
///
/// Each of these fields affects the internal state and thus changes how
/// subsequent fields (and messages) are interpreted.
///
/// # Naming conventions
/// Implementors of this trait should start with `TagLookup`.
pub trait TagLookup {
    type Error: Debug;

    fn from_dict(dict: &Dictionary) -> Self;

    /// Returns the [`BaseType`] of the tag number `tag`.
    fn lookup(&mut self, tag: u32) -> Result<DataType, Self::Error>;
}

/// A [`TagLookup`](TagLookup) that only allows a specific revision of the
/// standard, like most venues do.
#[derive(Debug)]
pub struct TagLookupPredetermined {
    current_dict: Rc<Dictionary>,
}

impl TagLookup for TagLookupPredetermined {
    type Error = TagLookupPredeterminedError;

    fn from_dict(dict: &Dictionary) -> Self {
        Self {
            current_dict: Rc::new(dict.clone()),
        }
    }

    fn lookup(&mut self, tag: u32) -> Result<DataType, Self::Error> {
        // TODO
        match tag {
            // `ApplVerID <1128>`
            1128 => {}
            // `ApplExtID <1156>`
            1156 => {
                return Err(Self::Error::InvalidApplExtID);
            }
            // `CstmApplVerID <1129>`
            1129 => {
                return Err(Self::Error::InvalidCstmApplVerID);
            }
            // `DefaultApplVerID <1137>`
            1137 => {
                return Err(Self::Error::InvalidApplExtID);
            }
            // `DefaultApplExtID <1407>`
            1407 => {
                return Err(Self::Error::InvalidApplExtID);
            }
            // `DefaultCstmApplVerID <1408>`
            1408 => {
                return Err(Self::Error::InvalidCstmApplVerID);
            }
            _ => (),
        };
        Ok(self
            .current_dict
            .field_by_tag(tag)
            .map(|f| f.basetype())
            .unwrap_or(DataType::String))
    }
}

#[derive(Debug)]
pub enum TagLookupPredeterminedError {
    InvalidApplVerID,
    InvalidApplExtID,
    InvalidCstmApplVerID,
}

struct FieldIter<'a, Z: Config> {
    data: &'a [u8],
    cursor: usize,
    config: Z,
    tag_lookup: Z::TagLookup,
    data_field_length: usize,
}

impl<'a, Z> FieldIter<'a, Z>
where
    Z: Config,
{
    fn new(data: &'a [u8], config: Z, dictionary: &'a Dictionary) -> Self {
        Self {
            data,
            cursor: 0,
            config,
            tag_lookup: Z::TagLookup::from_dict(dictionary),
            data_field_length: 0,
        }
    }
}

impl<'a, Z> Iterator for &mut FieldIter<'a, Z>
where
    Z: Config,
{
    type Item = Result<slr::Field, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.data.len() {
            return None;
        }
        let mut tag = 0u32;
        while let Some(byte) = self.data.get(self.cursor) {
            self.cursor += 1;
            if *byte == b'=' {
                if tag == 0 {
                    return Some(Err(DecodeError::Syntax));
                } else {
                    break;
                }
            }
            tag = tag * 10 + byte.wrapping_sub(b'0') as u32;
        }
        if self.data.get(self.cursor).is_none() {
            return Some(Err(DecodeError::Syntax));
        }
        debug_assert_eq!(self.data[self.cursor - 1], b'=');
        debug_assert!(tag > 0);
        let datatype = self.tag_lookup.lookup(tag);
        dbglog!("Parsing a field with data type '{:?}'.", &datatype);
        let mut field_value = FixFieldValue::from(0i64);
        match datatype {
            Ok(DataType::Data) => {
                field_value = FixFieldValue::Atom(val::Atomic::Data(
                    self.data[self.cursor..self.cursor + self.data_field_length].to_vec(),
                ));
                self.cursor += self.data_field_length + 1;
                debug_assert_eq!(self.data[self.cursor - 1], self.config.separator());
            }
            Ok(datatype) => {
                dbglog!(
                    "Parsing the field value of <{}> (residual data as lossy UTF-8 is '{}').",
                    tag,
                    String::from_utf8_lossy(&self.data[self.cursor..]),
                );
                if let Some(separator_i) = &self.data[self.cursor..]
                    .iter()
                    .position(|byte| *byte == self.config.separator())
                    .map(|i| i + self.cursor)
                {
                    field_value =
                        read_field_value(datatype, &self.data[self.cursor..*separator_i]).unwrap();
                    self.cursor = separator_i + 1;
                    debug_assert_eq!(self.data[self.cursor - 1], self.config.separator());
                    if datatype == DataType::Length {
                        self.data_field_length = field_value.as_length().unwrap();
                    }
                } else {
                    dbglog!("EOF before expected separator. Error.");
                    return Some(Err(DecodeError::Syntax));
                }
            }
            Err(_) => (),
        }
        debug_assert_eq!(self.data[self.cursor - 1], self.config.separator());
        Some(Ok(slr::Field::new(tag, field_value)))
    }
}

fn read_field_value(datatype: DataType, buf: &[u8]) -> Result<FixFieldValue, DecodeError> {
    debug_assert!(!buf.is_empty());
    Ok(match datatype {
        DataType::Char => FixFieldValue::from(buf[0] as char),
        DataType::Data => FixFieldValue::Atom(val::Atomic::Data(buf.to_vec())),
        DataType::Float => FixFieldValue::Atom(val::Atomic::float(
            str::from_utf8(buf)
                .map_err(|_| DecodeError::Syntax)?
                .parse::<f32>()
                .map_err(|_| DecodeError::Syntax)?,
        )),
        DataType::Int => {
            let mut n = 0i64;
            let mut multiplier = 1;
            for byte in buf.iter().rev() {
                if *byte >= b'0' && *byte <= b'9' {
                    let digit = byte - b'0';
                    n += digit as i64 * multiplier;
                } else if *byte == b'-' {
                    n *= -1;
                } else if *byte != b'+' {
                    return Err(DecodeError::Syntax);
                }
                multiplier *= 10;
            }
            FixFieldValue::from(n)
        }
        _ => FixFieldValue::String(
            str::from_utf8(buf)
                .map_err(|_| DecodeError::Syntax)?
                .to_string(),
        ),
    })
}

/// The [`Config`](Config) pattern allows deep customization of encoding
/// and decoding behavior without relying on runtime settings. By using this
/// trait and specializing the behavior of particular methods, users can change
/// the behavior of the FIX encoder without incurring in performance loss.
///
/// # Naming conventions
/// Implementors of this trait should start with `Config`.
pub trait Config: Clone + Default {
    type TagLookup: TagLookup;

    /// The delimiter character, which terminates every tag-value pair including
    /// the last one.
    ///
    /// ASCII 0x1 (SOH) is the default separator character.
    #[inline]
    fn separator(&self) -> u8 {
        0x1
    }

    #[inline]
    fn max_message_size(&self) -> Option<usize> {
        Some(65536)
    }

    #[inline]
    #[deprecated(note = "BodyLength is mandatory. This method is ignored.")]
    fn verify_body_length(&self) -> bool {
        true
    }

    #[inline]
    fn verify_checksum(&self) -> bool {
        true
    }
}

/// A [`Config`] for [`Codec`] with default configuration
/// options.
///
/// This configurator uses [`ChecksumAlgoDefault`] as a checksum algorithm and
/// [`TagLookupPredetermined`] for its dynamic tag lookup logic.
#[derive(Debug, Default, Clone)]
pub struct ConfigFastDefault;

impl Config for ConfigFastDefault {
    type TagLookup = TagLookupPredetermined;
}

#[derive(Debug, Clone)]
pub struct ConfigSettable {
    separator: u8,
    verify_checksum: bool,
}

impl ConfigSettable {
    pub fn with_separator(mut self, separator: u8) -> Self {
        self.separator = separator;
        self
    }

    pub fn with_verify_checksum(mut self, verify: bool) -> Self {
        self.verify_checksum = verify;
        self
    }
}

impl Config for ConfigSettable {
    type TagLookup = TagLookupPredetermined;

    #[inline]
    fn separator(&self) -> u8 {
        self.separator
    }

    #[inline]
    fn verify_checksum(&self) -> bool {
        self.verify_checksum
    }
}

impl Default for ConfigSettable {
    fn default() -> Self {
        Self {
            separator: b'|',
            verify_checksum: true,
        }
    }
}

type EncodeError = ();

#[derive(Clone, Debug, PartialEq)]
pub enum DecodeError {
    FieldPresence,
    Syntax,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SuperError is here!")
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<io::Error> for DecodeError {
    fn from(_err: io::Error) -> Self {
        Self::Syntax // FIXME
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Use http://www.validfix.com/fix-analyzer.html for testing.

    fn encoder() -> Codec<slr::Message, impl Config> {
        let config = ConfigSettable::default().with_separator(b'|');
        Codec::new(config)
    }

    fn encoder_with_soh() -> Codec<slr::Message, impl Config> {
        Codec::new(ConfigFastDefault)
    }

    fn encoder_slash_no_verify() -> Codec<slr::Message, impl Config> {
        let config = ConfigSettable::default()
            .with_separator(b'|')
            .with_verify_checksum(false);
        Codec::new(config)
    }

    fn with_soh(msg: &str) -> String {
        msg.split("|").collect::<Vec<&str>>().join("\x01")
    }

    #[test]
    fn can_parse_simple_message() {
        let message = "8=FIX.4.2|9=40|35=D|49=AFUNDMGR|56=ABROKER|15=USD|59=0|10=091|";
        let config = ConfigSettable::default().with_separator(b'|');
        let mut codec = Codec::<slr::Message, _>::new(config);
        let result = codec.decode(message.as_bytes());
        assert!(result.is_ok());
    }

    const RANDOM_MESSAGES: &[&str] = &[
        "8=FIX.4.2|9=42|35=0|49=A|56=B|34=12|52=20100304-07:59:30|10=185|",
        "8=FIX.4.2|9=97|35=6|49=BKR|56=IM|34=14|52=20100204-09:18:42|23=115685|28=N|55=SPMI.MI|54=2|44=2200.75|27=S|25=H|10=248|",
        "8=FIX.4.4|9=117|35=AD|34=2|49=A|50=1|52=20100219-14:33:32.258|56=B|57=M|263=1|568=1|569=0|580=1|75=20100218|60=20100218-00:00:00.000|10=202|",
        "8=FIX.4.4|9=94|35=3|34=214|49=A|50=U1|52=20100304-09:42:23.130|56=AB|128=B1|45=176|58=txt|371=15|372=X|373=1|10=058|",
        "8=FIX.4.4|9=70|35=4|49=A|56=XYZ|34=129|52=20100302-19:38:21|43=Y|57=LOL|123=Y|36=175|10=192|",
        "8=FIX.4.4|9=122|35=D|34=215|49=CLIENT12|52=20100225-19:41:57.316|56=B|1=Marcel|11=13346|21=1|40=2|44=5|54=1|59=0|60=20100225-19:39:52.020|10=072|",
        "8=FIX.4.2|9=196|35=X|49=A|56=B|34=12|52=20100318-03:21:11.364|262=A|268=2|279=0|269=0|278=BID|55=EUR/USD|270=1.37215|15=EUR|271=2500000|346=1|279=0|269=1|278=OFFER|55=EUR/USD|270=1.37224|15=EUR|271=2503200|346=1|10=171|",
    ];

    #[test]
    fn skip_checksum_verification() {
        let message = "8=FIX.FOOBAR|9=5|35=0|10=000|";
        let config = ConfigSettable::default()
            .with_separator(b'|')
            .with_verify_checksum(false);
        let mut codec = Codec::<slr::Message, _>::new(config);
        let result = codec.decode(message.as_bytes());
        assert!(result.is_ok());
    }

    #[test]
    fn no_skip_checksum_verification() {
        let message = "8=FIX.FOOBAR|9=5|35=0|10=000|";
        let config = ConfigSettable::default()
            .with_separator(b'|')
            .with_verify_checksum(true);
        let mut codec = Codec::<slr::Message, _>::new(config);
        let result = codec.decode(message.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn assortment_of_random_messages_is_ok() {
        for msg_with_vertical_bar in RANDOM_MESSAGES {
            let message = with_soh(msg_with_vertical_bar);
            let mut codec = encoder_with_soh();
            let result = codec.decode(message.as_bytes());
            result.unwrap();
        }
    }

    #[test]
    fn heartbeat_message_fields_are_ok() {
        let mut codec = encoder_slash_no_verify();
        let message = codec.decode(&mut RANDOM_MESSAGES[0].as_bytes()).unwrap();
        assert_eq!(
            message.get_field(8),
            Some(&FixFieldValue::String("FIX.4.2".to_string()))
        );
        assert_eq!(
            message.get_field(35),
            Some(&FixFieldValue::String("0".to_string()))
        );
    }

    #[test]
    fn message_without_final_separator() {
        let message = "8=FIX.4.4|9=122|35=D|34=215|49=CLIENT12|52=20100225-19:41:57.316|56=B|1=Marcel|11=13346|21=1|40=2|44=5|54=1|59=0|60=20100225-19:39:52.020|10=072";
        let config = ConfigSettable::default().with_separator(b'|');
        let mut codec = Codec::<slr::Message, _>::new(config);
        let result = codec.decode(message.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn message_must_end_with_separator() {
        let msg = "8=FIX.4.2|9=41|35=D|49=AFUNDMGR|56=ABROKERt|15=USD|59=0|10=127";
        let mut codec = encoder();
        let result = codec.decode(&mut msg.as_bytes());
        assert_eq!(result, Err(DecodeError::Syntax));
    }

    #[test]
    fn message_without_checksum() {
        let msg = "8=FIX.4.4|9=37|35=D|49=AFUNDMGR|56=ABROKERt|15=USD|59=0|";
        let mut codec = encoder();
        let result = codec.decode(&mut msg.as_bytes());
        assert_eq!(result, Err(DecodeError::Syntax));
    }

    #[test]
    fn message_without_standard_header() {
        let msg = "35=D|49=AFUNDMGR|56=ABROKERt|15=USD|59=0|10=000|";
        let mut codec = encoder();
        let result = codec.decode(&mut msg.as_bytes());
        assert_eq!(result, Err(DecodeError::Syntax));
    }

    #[test]
    fn detect_incorrect_checksum() {
        let msg = "8=FIX.4.2|9=43|35=D|49=AFUNDMGR|56=ABROKER|15=USD|59=0|10=146|";
        let mut codec = encoder();
        let result = codec.decode(&mut msg.as_bytes());
        assert_eq!(result, Err(DecodeError::Syntax));
    }

    #[test]
    fn agnostic_simple_message() {
        let msg = "8=FIX.4.2|9=40|35=D|49=AFUNDMGR|56=ABROKER|15=USD|59=0|10=091|";
        let config = ConfigSettable::default().with_separator(b'|');
        let mut decoder = CodecAgnostic::<ConfigSettable>::default();
        *decoder.config_mut() = config;
        let message = decoder.decode(&mut msg.as_bytes()).unwrap();
        assert_eq!(message.field_begin_string(), b"FIX.4.2");
        assert_eq!(message.body(), b"35=D|49=AFUNDMGR|56=ABROKER|15=USD|59=0|");
    }

    #[test]
    fn agnostic_empty_body() {
        let msg = "8=FIX.FOOBAR|9=0|10=225|";
        let config = ConfigSettable::default().with_separator(b'|');
        let mut decoder = CodecAgnostic::<ConfigSettable>::default();
        *decoder.config_mut() = config;
        let message = decoder.decode(&mut msg.as_bytes()).unwrap();
        assert_eq!(message.field_begin_string(), b"FIX.FOOBAR");
        assert_eq!(message.body(), b"");
    }

    #[test]
    fn agnostic_edge_cases_no_panic() {
        let config = ConfigSettable::default().with_separator(b'|');
        let mut decoder = CodecAgnostic::<ConfigSettable>::default();
        *decoder.config_mut() = config;
        decoder.decode(b"8=FIX.FOOBAR|9=0|10=225|").ok();
        decoder.decode(b"8=|9=0|10=225|").ok();
        decoder.decode(b"8=|9=0|10=|").ok();
        decoder.decode(b"8====|9=0|10=|").ok();
        decoder.decode(b"|||9=0|10=|").ok();
        decoder.decode(b"9999999999999").ok();
        decoder.decode(b"-9999999999999").ok();
        decoder.decode(b"==============").ok();
        decoder.decode(b"9999999999999|").ok();
        decoder.decode(b"|999999999999=|").ok();
        decoder.decode(b"|999=999999999999999999|=").ok();
    }
}
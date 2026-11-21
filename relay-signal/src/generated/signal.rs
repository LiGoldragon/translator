#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_extern_crates,
    clippy::type_complexity
)]
#[rustfmt::skip]
extern crate alloc;
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct Name(pub alloc::string::String);
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Hook {
    Started(Name),
    Ended(Name),
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Notice {
    Hooked(Hook),
    Lined(relay::emission::Line),
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub struct Call {
    pub caller: translator_transport::Caller,
    pub input: Input,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Input {
    Run(relay::emission::Command),
    Follow(relay::emission::Follow),
    Report(Hook),
    Seen,
    Listening,
}
#[rustfmt::skip]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "datom", derive(datom_codec::Datomizable, datom_codec::Composing))]
pub enum Output {
    Ran(relay::emission::Finished),
    Following,
    Reported,
    Noticed(alloc::vec::Vec<Notice>),
    Acknowledged,
    Unobserved(translator_transport::Unobserved),
}
#[rustfmt::skip]
#[allow(clippy::result_large_err, clippy::ptr_arg, clippy::borrowed_box)]
pub trait Answering {
    fn run(
        &self,
        command: &relay::emission::Command,
    ) -> core::result::Result<relay::emission::Finished, Output>;
    fn follow(
        &self,
        follow: &relay::emission::Follow,
    ) -> core::result::Result<(), Output>;
    fn report(&self, hook: &Hook) -> core::result::Result<(), Output>;
    fn seen(&self) -> core::result::Result<alloc::vec::Vec<Notice>, Output>;
    fn listening(&self) -> core::result::Result<(), Output>;
}
#[rustfmt::skip]
pub trait Answerable {
    type Answer;
    fn answer<Nexus: Answering>(&self, nexus: &Nexus) -> Self::Answer;
}
#[rustfmt::skip]
impl Answerable for Input {
    type Answer = Output;
    fn answer<Nexus: Answering>(&self, nexus: &Nexus) -> Output {
        match self {
            Input::Run(carried) => {
                nexus.run(carried).map_or_else(|refusal| refusal, Output::Ran)
            }
            Input::Follow(carried) => {
                nexus
                    .follow(carried)
                    .map_or_else(|refusal| refusal, |()| Output::Following)
            }
            Input::Report(carried) => {
                nexus
                    .report(carried)
                    .map_or_else(|refusal| refusal, |()| Output::Reported)
            }
            Input::Seen => nexus.seen().map_or_else(|refusal| refusal, Output::Noticed),
            Input::Listening => {
                nexus
                    .listening()
                    .map_or_else(|refusal| refusal, |()| Output::Acknowledged)
            }
        }
    }
}

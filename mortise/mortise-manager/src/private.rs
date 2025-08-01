use crate::{MortiseObject, MortiseOpenObject};
pub trait MortiseSealed {}
impl MortiseSealed for MortiseObject {}
impl MortiseSealed for MortiseOpenObject {}

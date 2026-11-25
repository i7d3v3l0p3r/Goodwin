#[cfg(target_os = "espidf")]
pub(crate) mod espidf;
#[cfg(not(target_os = "espidf"))]
pub(crate) mod linux;

#[cfg(target_os = "espidf")]
pub(crate) use espidf::run;
#[cfg(not(target_os = "espidf"))]
pub(crate) use linux::run;

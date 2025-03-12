use asr::{Address, FromEndian, Process};
use bytemuck::CheckedBitPattern;

pub(crate) fn read_host_path<T: CheckedBitPattern>(
    process: &Process,
    base_address: Address,
    offsets: &[u32],
) -> Option<T> {
    let mut address = base_address;

    let (&last, path) = offsets.split_last()?;

    for &offset in path {
        let uaddress = process.read::<u32>(address + offset).ok()?.from_be();
        address = base_address + uaddress;
    }

    process.read::<T>(address + last).ok()
}

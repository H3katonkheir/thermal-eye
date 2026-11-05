//! Which logical processors share a physical core and an L3 cache. On Ryzen an
//! L3 domain is a CCD, which is how cores are matched to their die temperature.

use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::System::SystemInformation::{
    GROUP_AFFINITY, GetLogicalProcessorInformationEx, RelationAll, RelationCache,
    RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};

#[derive(Clone, Debug, Default)]
pub struct L3Domain {
    pub processors: Vec<usize>,
    pub size_mb: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Topology {
    /// Logical processors of each physical core, ordered by first processor.
    pub cores: Vec<Vec<usize>>,
    /// L3 domains, ordered by first processor.
    pub l3: Vec<L3Domain>,
}

impl Topology {
    pub fn read() -> Self {
        read().unwrap_or_default()
    }

    pub fn core_of(&self, processor: usize) -> Option<usize> {
        self.cores.iter().position(|core| core.contains(&processor))
    }

    pub fn l3_of(&self, processor: usize) -> Option<usize> {
        self.l3.iter().position(|domain| domain.processors.contains(&processor))
    }
}

fn expand(affinity: &GROUP_AFFINITY) -> Vec<usize> {
    let base = affinity.Group as usize * 64;
    (0..64).filter(|bit| affinity.Mask >> bit & 1 == 1).map(|bit| base + bit).collect()
}

fn read() -> Option<Topology> {
    let mut length = 0u32;
    unsafe {
        let probe = GetLogicalProcessorInformationEx(RelationAll, None, &mut length);
        if probe.err().map(|e| e.code()) != Some(ERROR_INSUFFICIENT_BUFFER.to_hresult()) {
            return None;
        }
        // u64 storage keeps the records aligned.
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        let base = buffer.as_mut_ptr().cast::<u8>();
        GetLogicalProcessorInformationEx(RelationAll, Some(base.cast()), &mut length).ok()?;

        let mut topology = Topology::default();
        let mut offset = 0usize;
        while offset < length as usize {
            let record = &*base.add(offset).cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
            if record.Relationship == RelationProcessorCore {
                let processor = &record.Anonymous.Processor;
                let groups = std::slice::from_raw_parts(
                    processor.GroupMask.as_ptr(),
                    processor.GroupCount as usize,
                );
                topology.cores.push(groups.iter().flat_map(expand).collect());
            } else if record.Relationship == RelationCache && record.Anonymous.Cache.Level == 3 {
                let cache = &record.Anonymous.Cache;
                topology.l3.push(L3Domain {
                    processors: expand(&cache.Anonymous.GroupMask),
                    size_mb: cache.CacheSize / (1024 * 1024),
                });
            }
            offset += record.Size as usize;
        }
        topology.cores.sort_by_key(|core| core.first().copied());
        topology.l3.sort_by_key(|domain| domain.processors.first().copied());
        Some(topology)
    }
}

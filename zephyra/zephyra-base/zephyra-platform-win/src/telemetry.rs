use std::mem::{size_of, MaybeUninit};
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_CSTATUS_NO_INSTANCE, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PDH_NO_DATA,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessTimes};

const CPU_TICKS_PER_SECOND: f64 = 10_000_000.0;

#[derive(Default)]
struct GpuSample {
    engine_percent: Option<f64>,
    dedicated_bytes: Option<u64>,
    shared_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessResourceSample {
    pub process_id: u32,
    pub cpu_percent: Option<f64>,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    pub gpu_engine_percent: Option<f64>,
    pub gpu_dedicated_bytes: Option<u64>,
    pub gpu_shared_bytes: Option<u64>,
}

impl ProcessResourceSample {
    pub fn to_text(&self, gpu_error: Option<&str>) -> String {
        let cpu = self
            .cpu_percent
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "warming_up".into());
        let gpu_value = |value: Option<f64>| match (value, gpu_error) {
            (Some(value), _) => format!("{value:.2}"),
            (None, Some(error)) => format!("unavailable ({error})"),
            (None, None) => "warming_up_or_idle".into(),
        };
        let gpu_memory = |value: Option<u64>| match (value, gpu_error) {
            (Some(value), _) => value.to_string(),
            (None, Some(error)) => format!("unavailable ({error})"),
            (None, None) => "warming_up_or_idle".into(),
        };
        format!(
            concat!(
                "process_id={}\n",
                "cpu_process_percent={}\n",
                "cpu_basis=whole_machine_capacity\n",
                "memory_working_set_bytes={}\n",
                "memory_private_bytes={}\n",
                "gpu_engine_max_percent={}\n",
                "gpu_dedicated_bytes={}\n",
                "gpu_shared_bytes={}"
            ),
            self.process_id,
            cpu,
            self.working_set_bytes,
            self.private_bytes,
            gpu_value(self.gpu_engine_percent),
            gpu_memory(self.gpu_dedicated_bytes),
            gpu_memory(self.gpu_shared_bytes),
        )
    }
}

#[derive(Debug)]
pub struct ProcessTelemetry {
    process_id: u32,
    query: Option<PDH_HQUERY>,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_dedicated: Option<PDH_HCOUNTER>,
    gpu_shared: Option<PDH_HCOUNTER>,
    gpu_error: Option<String>,
    last_cpu: Option<(u64, Instant)>,
}

impl ProcessTelemetry {
    pub fn new() -> Self {
        let process_id = unsafe { GetCurrentProcessId() };
        let mut telemetry = Self {
            process_id,
            query: None,
            gpu_engine: None,
            gpu_dedicated: None,
            gpu_shared: None,
            gpu_error: None,
            last_cpu: None,
        };
        telemetry.open_gpu_counters();
        telemetry
    }

    pub fn sample(&mut self) -> Result<ProcessResourceSample, String> {
        let now = Instant::now();
        let cpu_time = process_cpu_ticks()?;
        let cpu_percent = self.last_cpu.and_then(|(previous_ticks, previous_at)| {
            let elapsed = now.duration_since(previous_at).as_secs_f64();
            if elapsed == 0.0 {
                return None;
            }
            let cpu_elapsed = cpu_time.saturating_sub(previous_ticks) as f64 / CPU_TICKS_PER_SECOND;
            let logical_cpus = std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1) as f64;
            Some((cpu_elapsed / elapsed / logical_cpus * 100.0).clamp(0.0, 100.0))
        });
        self.last_cpu = Some((cpu_time, now));

        let mut memory = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
                memory.cb,
            )
        }
        .map_err(|error| format!("读取进程内存失败: {error}"))?;

        let gpu = match self.sample_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                self.gpu_error = Some(error);
                GpuSample::default()
            }
        };
        Ok(ProcessResourceSample {
            process_id: self.process_id,
            cpu_percent,
            working_set_bytes: memory.WorkingSetSize as u64,
            private_bytes: memory.PrivateUsage as u64,
            gpu_engine_percent: gpu.engine_percent,
            gpu_dedicated_bytes: gpu.dedicated_bytes,
            gpu_shared_bytes: gpu.shared_bytes,
        })
    }

    pub fn gpu_error(&self) -> Option<&str> {
        self.gpu_error.as_deref()
    }

    fn open_gpu_counters(&mut self) {
        let mut query = PDH_HQUERY(std::ptr::null_mut());
        let status = unsafe { PdhOpenQueryW(None, 0, &mut query) };
        if status != 0 {
            self.gpu_error = Some(format!("PdhOpenQueryW failed with status 0x{status:08X}"));
            return;
        }

        let mut engine = PDH_HCOUNTER(std::ptr::null_mut());
        let mut dedicated = PDH_HCOUNTER(std::ptr::null_mut());
        let mut shared = PDH_HCOUNTER(std::ptr::null_mut());
        let counters = [
            (w!(r"\GPU Engine(*)\Utilization Percentage"), &mut engine),
            (
                w!(r"\GPU Process Memory(*)\Dedicated Usage"),
                &mut dedicated,
            ),
            (w!(r"\GPU Process Memory(*)\Shared Usage"), &mut shared),
        ];
        for (path, handle) in counters {
            let status = unsafe { PdhAddEnglishCounterW(query, path, 0, handle) };
            if status != 0 {
                unsafe {
                    PdhCloseQuery(query);
                }
                self.gpu_error = Some(format!(
                    "PdhAddEnglishCounterW failed with status 0x{status:08X}"
                ));
                return;
            }
        }
        self.query = Some(query);
        self.gpu_engine = Some(engine);
        self.gpu_dedicated = Some(dedicated);
        self.gpu_shared = Some(shared);
    }

    fn sample_gpu(&mut self) -> Result<GpuSample, String> {
        let Some(query) = self.query else {
            return Ok(GpuSample::default());
        };
        let status = unsafe { PdhCollectQueryData(query) };
        if status == PDH_NO_DATA {
            return Ok(GpuSample::default());
        }
        if status != 0 {
            return Err(format!(
                "PdhCollectQueryData failed with status 0x{status:08X}"
            ));
        }

        let gpu_engine = self
            .gpu_engine
            .map(|counter| counter_values(counter, self.process_id))
            .transpose()?
            .unwrap_or_default();
        let dedicated = self
            .gpu_dedicated
            .map(|counter| counter_values(counter, self.process_id))
            .transpose()?
            .unwrap_or_default();
        let shared = self
            .gpu_shared
            .map(|counter| counter_values(counter, self.process_id))
            .transpose()?
            .unwrap_or_default();

        Ok(GpuSample {
            engine_percent: gpu_engine
                .into_iter()
                .map(|(_, value)| value)
                .map(|value| value.clamp(0.0, 100.0))
                .reduce(f64::max),
            dedicated_bytes: dedicated
                .into_iter()
                .map(|(_, value)| value.max(0.0) as u64)
                .reduce(u64::saturating_add),
            shared_bytes: shared
                .into_iter()
                .map(|(_, value)| value.max(0.0) as u64)
                .reduce(u64::saturating_add),
        })
    }
}

impl Default for ProcessTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ProcessTelemetry {
    fn drop(&mut self) {
        if let Some(query) = self.query {
            unsafe {
                PdhCloseQuery(query);
            }
        }
    }
}

fn process_cpu_ticks() -> Result<u64, String> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(|error| format!("读取进程 CPU 时间失败: {error}"))?;
    Ok(filetime_ticks(kernel).saturating_add(filetime_ticks(user)))
}

fn filetime_ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn counter_values(counter: PDH_HCOUNTER, process_id: u32) -> Result<Vec<(String, f64)>, String> {
    let mut bytes = 0_u32;
    let mut count = 0_u32;
    let status = unsafe {
        PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, None)
    };
    if status == PDH_CSTATUS_NO_INSTANCE || status == PDH_NO_DATA {
        return Ok(Vec::new());
    }
    if status != PDH_MORE_DATA {
        return Err(format!(
            "PdhGetFormattedCounterArrayW(size) failed with status 0x{status:08X}"
        ));
    }
    let capacity = (bytes as usize).div_ceil(size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
    let mut items = Vec::<MaybeUninit<PDH_FMT_COUNTERVALUE_ITEM_W>>::with_capacity(capacity);
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut bytes,
            &mut count,
            Some(items.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()),
        )
    };
    if status == PDH_CSTATUS_NO_INSTANCE || status == PDH_NO_DATA {
        return Ok(Vec::new());
    }
    if status != 0 {
        return Err(format!(
            "PdhGetFormattedCounterArrayW(data) failed with status 0x{status:08X}"
        ));
    }
    unsafe {
        items.set_len(count as usize);
    }
    let process_marker = format!("pid_{process_id}_");
    let mut values = Vec::new();
    for item in items {
        let item = unsafe { item.assume_init() };
        let name = unsafe { item.szName.to_string() }
            .map_err(|error| format!("GPU counter instance name is invalid: {error}"))?;
        if name.contains(&process_marker) && item.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA {
            let value = unsafe { item.FmtValue.Anonymous.doubleValue };
            if value.is_finite() {
                values.push((name, value));
            }
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::{ProcessResourceSample, ProcessTelemetry};

    #[test]
    fn resource_text_reports_values_and_warming_status() {
        let sample = ProcessResourceSample {
            process_id: 42,
            cpu_percent: Some(12.5),
            working_set_bytes: 1024,
            private_bytes: 512,
            gpu_engine_percent: None,
            gpu_dedicated_bytes: None,
            gpu_shared_bytes: None,
        };
        let text = sample.to_text(None);
        assert!(text.contains("process_id=42"));
        assert!(text.contains("cpu_process_percent=12.50"));
        assert!(text.contains("memory_working_set_bytes=1024"));
        assert!(text.contains("memory_private_bytes=512"));
        assert!(text.contains("gpu_engine_max_percent=warming_up_or_idle"));
    }

    #[test]
    fn resource_text_preserves_gpu_unavailable_reason() {
        let sample = ProcessResourceSample {
            process_id: 42,
            cpu_percent: None,
            working_set_bytes: 0,
            private_bytes: 0,
            gpu_engine_percent: None,
            gpu_dedicated_bytes: None,
            gpu_shared_bytes: None,
        };
        let text = sample.to_text(Some("driver unavailable"));
        assert!(text.contains("cpu_process_percent=warming_up"));
        assert!(text.contains("gpu_engine_max_percent=unavailable (driver unavailable)"));
        assert!(text.contains("gpu_shared_bytes=unavailable (driver unavailable)"));
    }

    #[test]
    fn samples_current_process_memory_on_this_machine() {
        let mut telemetry = ProcessTelemetry::new();
        let first = telemetry.sample().unwrap();
        assert_ne!(first.process_id, 0);
        assert!(first.working_set_bytes > 0);
        assert!(first.private_bytes > 0);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let second = telemetry.sample().unwrap();
        assert!(second
            .cpu_percent
            .is_some_and(|value| (0.0..=100.0).contains(&value)));
    }
}

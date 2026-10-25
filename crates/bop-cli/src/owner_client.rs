//! Private Moth wire client. Native process-domain authority remains in Moth.
//! A retained registry owns each controller independently of cancellable waiters.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

const FRAME_MAX: usize = 65536;
const CHUNK: usize = 16384;
#[derive(Clone, Debug)]
pub struct Completion { pub reason: u8, pub raw_status: Option<u32>, pub stdout: Vec<u8>, pub stderr: Vec<u8> }
impl Completion {
    pub fn exit_code(&self) -> i32 {
        if self.reason == 2 { return 124; }
        if self.reason != 0 { return 1; }
        match self.raw_status { Some(s) if s & 0x7f == 0 => ((s >> 8) & 255) as i32, _ => 1 }
    }
}
struct Entry { #[allow(dead_code)] pid: u32, cancel: AtomicBool, result: Mutex<Option<Result<Completion, String>>>, task: Mutex<JoinState> }
enum JoinState { Starting, Running(tokio::task::JoinHandle<()>), Done(Result<(),String>) }
struct WakeNothing;
impl std::task::Wake for WakeNothing { fn wake(self: Arc<Self>) {} }
fn join_ready(entry:&Entry)->Result<bool,String> {
    use std::future::Future;
    let mut slot=entry.task.lock().map_err(|_| "owner task poisoned")?;
    match &mut *slot {
        JoinState::Starting=>Ok(false),
        JoinState::Done(result)=>result.clone().map(|_|true),
        JoinState::Running(task)=>{
            // Poll in place: cancellation never removes a pending JoinHandle.
            let waker=std::task::Waker::from(Arc::new(WakeNothing));
            let mut cx=std::task::Context::from_waker(&waker);
            match std::pin::Pin::new(task).poll(&mut cx) {
                std::task::Poll::Pending=>Ok(false),
                std::task::Poll::Ready(result)=>{
                    let result=result.map_err(|e|format!("owner controller join: {e}"));
                    *slot=JoinState::Done(result.clone()); result.map(|_|true)
                }
            }
        }
    }
}
fn remove_confirmed(key:&str,entry:&Arc<Entry>)->Result<bool,String> {
    if !entry.result.lock().map_err(|_| "owner result poisoned")?.as_ref().is_some_and(Result::is_ok) { return Ok(false); }
    if !join_ready(entry)? { return Ok(false); }
    let mut map=registry().lock().map_err(|_| "owner registry poisoned")?;
    if map.get(key).is_some_and(|held|Arc::ptr_eq(held,entry)) { map.remove(key); }
    Ok(true)
}
static REGISTRY: OnceLock<Mutex<BTreeMap<String, Arc<Entry>>>> = OnceLock::new();
fn registry() -> &'static Mutex<BTreeMap<String, Arc<Entry>>> { REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new())) }
pub struct Handle { key: String, entry: Arc<Entry> }
impl Drop for Handle { fn drop(&mut self) { self.entry.cancel.store(true, Ordering::Release); } }
impl Handle {
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 { self.entry.pid }
    #[allow(dead_code)]
    pub fn cancel(&self) { self.entry.cancel.store(true, Ordering::Release); }
    #[allow(dead_code)]
    pub async fn observe(&self, bound: Duration) -> Result<Option<Completion>, String> {
        match tokio::time::timeout(bound, self.wait()).await { Ok(result)=>result.map(Some), Err(_)=>Ok(None) }
    }
    pub async fn wait(&self) -> Result<Completion, String> {
        loop {
            let result = self.entry.result.lock().map_err(|_| "owner result poisoned")?.clone();
            if let Some(result) = result { return result; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    /// Only confirmed completion can release registry ownership. Unknown is retained.
    pub async fn release(self) -> Result<Completion, String> {
        let result = self.wait().await?;
        while !remove_confirmed(&self.key,&self.entry)? { tokio::time::sleep(Duration::from_millis(10)).await; }
        Ok(result)
    }
}
#[derive(Debug)]
pub enum StartError { NotSpawned(String), Retained(String) }
impl std::fmt::Display for StartError {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { match self { Self::NotSpawned(s)|Self::Retained(s)=>f.write_str(s) } }
}
impl std::error::Error for StartError {}
impl From<String> for StartError { fn from(s:String)->Self { Self::NotSpawned(s) } }
impl From<&str> for StartError { fn from(s:&str)->Self { Self::NotSpawned(s.into()) } }
impl From<StartError> for String { fn from(e:StartError)->Self { e.to_string() } }
/// Startup failure before spawn is an ordinary error. Once spawned, all handles
/// and protocol failures belong to the retained controller, never kill_on_drop.
pub fn start(key: String, mut command: Command, input: Vec<u8>, cap: usize) -> Result<Handle, StartError> {
    if !cfg!(target_os = "freebsd") { return Err("strict Moth ownership requires FreeBSD; no fallback".into()); }
    if cap == 0 || cap > 67108864 { return Err("invalid owner capture cap".into()); }
    let mut map = registry().lock().map_err(|_| "owner registry poisoned")?;
    if map.contains_key(&key) { return Err("owner invocation already retained".into()); }
    command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(false);
    let child = command.spawn().map_err(|e| format!("owner spawn: {e}"))?;
    let entry = Arc::new(Entry { pid: child.id().unwrap_or(0), cancel: AtomicBool::new(false), result: Mutex::new(None), task: Mutex::new(JoinState::Starting) });
    map.insert(key.clone(), entry.clone());
    let worker = entry.clone();
    let task = tokio::spawn(async move {
        // Direct-child ownership stays outside the unwindable protocol future.
        // Dropping that future closes command input, requesting native cleanup.
        let mut child=child;
        let polled = CatchPoll::new(drive(&mut child, input, cap, &worker)).await;
        let result = match polled {
            Ok(result)=>result,
            Err(_)=>{
                loop { match child.wait().await { Ok(_)=>break, Err(_)=>tokio::time::sleep(Duration::from_millis(100)).await } }
                Err("owner controller panicked; receipt unconfirmed after retained reap".into())
            }
        };
        if let Ok(mut slot) = worker.result.lock() { *slot = Some(result); }
    });
    *entry.task.lock().unwrap_or_else(|poison|poison.into_inner()) = JoinState::Running(task);
    Ok(Handle { key, entry })
}
/// Call before the owning process/runtime exits. It cannot manufacture finite
/// cleanup for Unknown: retained invocations keep this barrier pending.
pub async fn retirement_barrier() {
    loop {
        let entries=registry().lock().map(|map|map.iter().map(|(key,value)|(key.clone(),value.clone())).collect::<Vec<_>>());
        if let Ok(entries)=entries {
            if entries.is_empty() { return; }
            for (key,entry) in entries {
                entry.cancel.store(true,Ordering::Release);
                // An abandoned waiter cannot pin an already confirmed owner.
                // Unknown or failed joins stay registered for honest quarantine.
                let _=remove_confirmed(&key,&entry);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn frame(kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len()+5);
    out.extend_from_slice(&((bytes.len()+1) as u32).to_be_bytes()); out.push(kind); out.extend_from_slice(bytes); out
}
#[derive(Default)]
struct Wire { ready: bool, start_sent: bool, started: bool, credit: usize, exit: Option<u32>, retired: Option<(u8,Option<u32>)>, out: Vec<u8>, err: Vec<u8> }
#[cfg(unix)]
fn terminal_status(raw:u32)->bool {
    use std::os::unix::process::ExitStatusExt;
    let status=std::process::ExitStatus::from_raw(raw as i32);
    status.code().is_some() || status.signal().is_some()
}
#[cfg(not(unix))]
fn terminal_status(_raw:u32)->bool { false } // strict owner is Unix-only
impl Wire {
    fn event(&mut self, kind: u8, data: &[u8], cap: usize) -> Result<(), String> {
        if self.retired.is_some() { return Err("event after retirement".into()); }
        match kind {
            1 if !self.ready && data == b"MOTHOWN1" => self.ready = true,
            2 if self.ready && self.start_sent && !self.started && data.is_empty() => self.started = true,
            3 if self.started && data.len()==4 => {
                let credit = u32::from_be_bytes(data.try_into().unwrap()) as usize;
                if credit==0 { return Err("invalid zero input credit".into()); }
                self.credit = self.credit.checked_add(credit).filter(|x| *x <= 32768).ok_or("invalid input credit")?;
            }
            0x20 | 0x21 if self.started && data.len() <= CHUNK => {
                let dst = if kind==0x20 { &mut self.out } else { &mut self.err };
                if dst.len().checked_add(data.len()).filter(|x| *x <= cap).is_none() { return Err("capture limit exceeded".into()); }
                dst.extend_from_slice(data);
            }
            0x22 if self.started && self.exit.is_none() && data.len()==4 => { let raw=u32::from_be_bytes(data.try_into().unwrap());
                if !terminal_status(raw) { return Err("nonterminal payload wait status".into()); }
                self.exit=Some(raw); }
            0x23 if self.ready && data.len()==6 && data[0]<=7 && data[1]<=1 => {
                let raw=u32::from_be_bytes(data[2..].try_into().unwrap());
                let status=if data[1]==1 { Some(raw) } else { if raw!=0 { return Err("nonzero absent status".into()); } None };
                if status != self.exit || (data[0]==0 && (!self.started || status.is_none())) { return Err("retirement status mismatch".into()); }
                self.retired=Some((data[0],status));
            }
            0x24 if data.len()<=4096 && std::str::from_utf8(data).is_ok() => { /* observation only; retain owner */ }
            0x7f if data.len()<=4096 && std::str::from_utf8(data).is_ok() => return Err("owner rejected acquisition".into()),
            _ => return Err("invalid owner event/state".into()),
        }
        Ok(())
    }
}
/// Catch a poll unwind while retaining the future until the wrapper is dropped.
/// No executor/runtime or native ownership implementation is introduced here.
struct CatchPoll<F: std::future::Future> { inner: std::pin::Pin<Box<F>> }
impl<F: std::future::Future> CatchPoll<F> { fn new(future:F)->Self { Self { inner:Box::pin(future) } } }
impl<F: std::future::Future> std::future::Future for CatchPoll<F> {
    type Output=Result<F::Output,Box<dyn std::any::Any+Send>>;
    fn poll(self:std::pin::Pin<&mut Self>, cx:&mut std::task::Context<'_>)->std::task::Poll<Self::Output> {
        let this=self.get_mut();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| this.inner.as_mut().poll(cx))) {
            Ok(std::task::Poll::Pending)=>std::task::Poll::Pending,
            Ok(std::task::Poll::Ready(value))=>std::task::Poll::Ready(Ok(value)),
            Err(panic)=>std::task::Poll::Ready(Err(panic)),
        }
    }
}
#[cfg(all(test,target_os="freebsd",feature="timeout-process-tree-fixture"))]
pub async fn test_catch_poll<F:std::future::Future>(future:F)->Result<F::Output,Box<dyn std::any::Any+Send>> {
    CatchPoll::new(future).await
}

async fn drive(child: &mut Child, input: Vec<u8>, cap: usize, entry: &Entry) -> Result<Completion,String> {
    let mut stdin=child.stdin.take(); let mut stdout=child.stdout.take();
    let mut failure: Option<String>=None;
    if stdin.is_none() || stdout.is_none() { failure=Some("owner control acquisition failed".into()); stdin=None; }
    let mut wire=Wire::default(); let mut incoming=Vec::new(); let mut scratch=[0u8;8192];
    let mut outgoing=Vec::new(); let mut offset=0usize; let mut input_offset=0usize; let mut input_written=0usize;
    let mut sent_start=false; let mut sent_cancel=false; let mut eof=stdout.is_none();
    let mut owner_status=None; let born=tokio::time::Instant::now();
    loop {
        if !wire.ready && born.elapsed()>Duration::from_secs(4) && failure.is_none() { failure=Some("owner readiness timeout".into()); }
        if wire.retired.is_some() { stdin=None; outgoing.clear(); offset=0; }
        if failure.is_some() { stdin=None; outgoing.clear(); offset=0; }
        if outgoing.is_empty() && stdin.is_some() {
            if entry.cancel.load(Ordering::Acquire) && !sent_cancel { outgoing=frame(0x13,&[]); sent_cancel=true; }
            else if wire.ready && !sent_start && !sent_cancel { outgoing=frame(0x10,&[]); sent_start=true; }
            else if wire.started && !sent_cancel && input_offset<input.len() && wire.credit>0 {
                let count=CHUNK.min(wire.credit).min(input.len()-input_offset);
                outgoing=frame(0x11,&input[input_offset..input_offset+count]); input_offset+=count; wire.credit-=count;
            }
        }
        if owner_status.is_none() { match child.try_wait() { Ok(s)=>owner_status=s, Err(e)=>{failure.get_or_insert(format!("owner wait: {e}"));} } }
        if let (Some(status), true) = (owner_status.as_ref(), eof) {
            drop(stdin.take());
            if !incoming.is_empty() { failure.get_or_insert("truncated owner frame".into()); }
            if let Some(error)=failure { return Err(error); }
            if !status.success() { return Err("owner exited unsuccessfully; retirement unconfirmed".into()); }
            let (reason,raw_status)=wire.retired.ok_or("owner exit/EOF without retirement")?;
            if reason==0 && input_written!=input.len() { return Err("normal retirement before complete input delivery".into()); }
            return Ok(Completion { reason,raw_status,stdout:wire.out,stderr:wire.err });
        }
        tokio::select! {
            read=async { stdout.as_mut().unwrap().read(&mut scratch).await }, if stdout.is_some() && !eof => {
                match read {
                    Ok(0)=>{eof=true;stdout=None;if wire.retired.is_none() { failure.get_or_insert("control EOF without retirement".into()); }},
                    Ok(n)=>{
                        if failure.is_some() { continue; }
                        incoming.extend_from_slice(&scratch[..n]);
                        loop {
                            if incoming.len()<4 { break; }
                            let len=u32::from_be_bytes(incoming[..4].try_into().unwrap()) as usize;
                            if len==0 || len>FRAME_MAX { failure=Some("invalid owner frame length".into()); incoming.clear();break; }
                            if incoming.len()<len+4 { break; }
                            if let Err(e)=wire.event(incoming[4],&incoming[5..len+4],cap) { failure=Some(e); }
                            incoming.drain(..len+4);
                            if failure.is_some() { incoming.clear();break; }
                        }
                    }
                    Err(e)=>{ failure=Some(format!("owner control read: {e}"));stdout=None;eof=true; }
                }
            }
            write=async { stdin.as_mut().unwrap().write(&outgoing[offset..]).await }, if stdin.is_some() && !outgoing.is_empty() => {
                match write {
                    Ok(0) => { failure=Some("owner control short write".into()); },
                    Ok(n) => {
                        offset+=n;
                        if offset==outgoing.len() {
                            if outgoing[4]==0x11 { input_written+=outgoing.len()-5; }
                            if outgoing[4]==0x10 { wire.start_sent=true; }
                            outgoing.clear(); offset=0;
                        }
                    },
                    Err(e) => { failure=Some(format!("owner control write: {e}")); }
                }
            }
            _=tokio::time::sleep(Duration::from_millis(25))=>{}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn started() -> Wire { let mut w=Wire::default(); w.event(1,b"MOTHOWN1",128).unwrap(); w.start_sent=true; w.event(2,&[],128).unwrap(); w }
    #[test] fn protocol_rejects_spoofed_order_and_duplicate_ready() {
        let mut w=Wire::default(); assert!(w.event(2,&[],128).is_err());
        assert!(w.event(1,b"not-the-version",128).is_err());
        w.event(1,b"MOTHOWN1",128).unwrap(); assert!(w.event(1,b"MOTHOWN1",128).is_err());
        assert!(w.event(2,&[],128).is_err(),"Started before complete Start write");
    }
    #[test] fn retirement_requires_matching_observed_exit_and_rejects_trailing_events() {
        let mut w=started(); assert!(w.event(0x23,&[0,1,0,0,0,0],128).is_err());
        w.event(0x22,&0u32.to_be_bytes(),128).unwrap();
        assert!(w.event(0x23,&[0,1,0,0,1,0],128).is_err());
        w.event(0x23,&[0,1,0,0,0,0],128).unwrap();
        assert!(w.event(0x20,b"forged after retirement",128).is_err());
    }
    #[test] fn protocol_caps_credit_and_capture_without_truncation() {
        let mut w=started(); w.event(3,&32768u32.to_be_bytes(),128).unwrap();
        assert!(w.event(3,&1u32.to_be_bytes(),128).is_err());
        w.event(0x20,&[b'x';128],128).unwrap();
        assert!(w.event(0x20,b"x",128).is_err()); assert_eq!(w.out.len(),128);
    }
    #[test] fn unknown_and_payload_exit_are_not_retirement() {
        let mut w=started();w.event(0x24,b"kernel cleanup pending",128).unwrap();
        w.event(0x22,&0u32.to_be_bytes(),128).unwrap();assert!(w.retired.is_none());
        assert!(w.event(0x22,&0u32.to_be_bytes(),128).is_err());
    }
    #[test] fn failed_retirement_cannot_become_success() {
        let mut w=started();w.event(0x22,&0u32.to_be_bytes(),128).unwrap();
        w.event(0x23,&[2,1,0,0,0,0],128).unwrap();
        let (reason,raw_status)=w.retired.unwrap();
        assert_eq!(Completion { reason,raw_status,stdout:vec![],stderr:vec![] }.exit_code(),124);
    }
    #[cfg(unix)]
    #[test]
    fn malformed_credit_diagnostics_and_nonterminal_exit_are_rejected() {
        let mut w=started();
        assert!(w.event(3,&0u32.to_be_bytes(),128).is_err());
        assert_eq!(w.credit,0);
        w.event(0x24,&vec![b'x';4096],128).unwrap();
        assert!(w.event(0x24,&vec![b'x';4097],128).is_err());
        assert!(w.event(0x24,&[255],128).is_err());
        assert_eq!(w.event(0x7f,b"acquisition rejected",128).unwrap_err(),"owner rejected acquisition");
        assert_eq!(w.event(0x7f,&vec![b'x';4097],128).unwrap_err(),"invalid owner event/state");
        assert_eq!(w.event(0x7f,&[255],128).unwrap_err(),"invalid owner event/state");
        // WIFSTOPPED and WIFCONTINUED are observations, never terminal proof.
        for raw in [(19u32<<8)|0x7f,0xffff] {
            assert!(w.event(0x22,&raw.to_be_bytes(),128).is_err());
            assert!(w.exit.is_none());
        }
        #[cfg(target_os="freebsd")]
        assert!(w.event(0x22,&0x13u32.to_be_bytes(),128).is_err(),"FreeBSD continued is not terminal");
        for raw in [0u32,7<<8,15,9|0x80] {
            let mut terminal=started();terminal.event(0x22,&raw.to_be_bytes(),128).unwrap();
            assert_eq!(terminal.exit,Some(raw));
        }
    }
    fn entry_with_result(result:Result<Completion,String>)->Arc<Entry> {
        Arc::new(Entry { pid:0,cancel:AtomicBool::new(false),result:Mutex::new(Some(result)),task:Mutex::new(JoinState::Done(Ok(()))) })
    }
    fn complete()->Completion { Completion {reason:0,raw_status:Some(0),stdout:vec![],stderr:vec![]} }
    #[tokio::test]
    async fn abandoned_waiter_harvest_waits_for_real_controller_join() {
        let key="unit-harvest-join".to_string();let entry=entry_with_result(Ok(complete()));
        let finish=Arc::new(tokio::sync::Notify::new());let worker=finish.clone();
        *entry.task.lock().unwrap()=JoinState::Running(tokio::spawn(async move { worker.notified().await; }));
        registry().lock().unwrap().insert(key.clone(),entry.clone());
        drop(Handle {key:key.clone(),entry:entry.clone()});
        assert!(entry.cancel.load(Ordering::Acquire));
        assert!(!remove_confirmed(&key,&entry).unwrap());
        assert!(registry().lock().unwrap().contains_key(&key));
        finish.notify_one();
        while !remove_confirmed(&key,&entry).unwrap() { tokio::task::yield_now().await; }
        assert!(!registry().lock().unwrap().contains_key(&key));
    }
    #[test]
    fn harvest_retains_unknown_and_does_not_remove_replacement_identity() {
        let key="unit-harvest-identity".to_string();
        let unknown=entry_with_result(Err("unconfirmed retirement".into()));
        registry().lock().unwrap().insert(key.clone(),unknown.clone());
        assert!(!remove_confirmed(&key,&unknown).unwrap());
        let old=entry_with_result(Ok(complete()));
        assert!(remove_confirmed(&key,&old).unwrap());
        assert!(Arc::ptr_eq(registry().lock().unwrap().get(&key).unwrap(),&unknown));
        registry().lock().unwrap().remove(&key); // unit fixture cleanup only
    }

}

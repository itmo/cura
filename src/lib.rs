#![warn(missing_docs)]
//! An attempt at creating an Arc-RwLock combination that is straightforward
//! to use and no hassle , instead of worrying about being fast and lean. 
//!
//! * cloning referefences works like Arc
//! * made for sharing objects between threads without worry
//! * locking things works like RwLock with write() or read()
//! * it spins a few times and then queues if a lock is not obtained
//! * miri seems to be happy , so i trust it doesnt leak too much memory etc.
//! * requires that everything you stick into it is Send+Sync
//! * no need to constantly .unwrap() things instead it will just
//!   block forever or blow up
//!
//! # Example
//! ```
//! use cura::Cura;
//! let t:i32=1;
//! let foo=Cura::new(t);
//! let a=foo.clone();
//! let b=foo.clone();
//!
//! {
//!     assert_eq!(*a.read(),1);
//!     {
//!         a.alter(|s|{
//!            Some(2)
//!         });
//!     }
//!     let lock=a.read();
//!     let v=*lock;
//!     assert_eq!(v,2)
//! }//lock dropped here
//! {
//!     (*b.write())+=1; //lock dropped here i think 
//! }
//!
//! assert_eq!((*a.read()),3);
//!
//! ```
use std::ops::{Deref,DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize,AtomicI32,AtomicU32};
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release,SeqCst};
use std::cell::UnsafeCell;
use std::thread::Thread;
use std::time::{SystemTime,UNIX_EPOCH};
const LOCKED:i32=-999;
const FREE:i32=0;
const LOCKQUEUE:u32=u32::MAX/2;

/// a sort of an Arc that will both readwrite lock , be easy to
/// handle and is cloneable
/// ```
/// use cura::Cura;
/// let s=Cura::new(1);
/// let a=s.clone();
///
/// ```
pub struct Cura<T: Sync + Send> {
    ptr: NonNull<CuraData<T>>,
}
struct CuraData<T: Sync + Send> {
    count: AtomicUsize,
    data: UnsafeCell<T>,
    lockcount:AtomicI32, //-999=writeĺock,0=free,>0 readlock count
    queuecount:AtomicU32, // number of threads,
    queuedata:UnsafeCell<QueueData>,
}
struct QueueData
{
    queue:*mut QueueLink,
    endqueue:*mut QueueLink,
}
impl QueueData
{
    ///
    /// queue stuff into end of queue
    ///
    fn enqueue(&mut self,t:LockType)
    {
        let link=Box::leak(Box::new(QueueLink::new(t)));    
        let next=self.endqueue;
        if next.is_null()
        {
            self.queue=link;
        }else{
            unsafe{(*next).next=link;}
        }
        self.endqueue=link;
    }
    fn dequeue(&mut self)
    {
        //  dequeue
        let me=self.queue;
        unsafe{self.queue=(*self.queue).next;}
        //  if we were the last
        if me==self.endqueue
        {
            self.endqueue=std::ptr::null_mut();
        }
        unsafe {
            drop(Box::from_raw(me));
        }
    }
}
#[derive(PartialEq)]
enum LockType
{
    Read,
    Write,
}
struct QueueLink
{
    thread:Thread,
    lock:LockType,
    next:*mut QueueLink,
}
impl QueueLink
{
    fn new(l:LockType)->QueueLink
    {
        QueueLink{
            thread:std::thread::current(),
            lock:l,
            next:std::ptr::null_mut(),
        }
    }
    /*
        useful methods
    */
}
///
/// Cura public interface
///
impl<T: Sync + Send> Cura<T> {
    ///
    /// constructor for a Cura 
    /// ```
    ///     use cura::Cura;
    ///     let t=1;
    ///     let foo=Cura::new(t); //instead of Arc::new(Mutex::new(t));
    /// ```
    pub fn new(t: T) -> Cura<T> {
        let queuedata=UnsafeCell::new(QueueData{
                queue:std::ptr::null_mut(),
                endqueue:std::ptr::null_mut(),
            });
        Cura {
            ptr: NonNull::from(Box::leak(Box::new(CuraData {
                count: AtomicUsize::new(1),
                data: UnsafeCell::new(t),
                lockcount:AtomicI32::new(0),
                queuecount:AtomicU32::new(0), //
                queuedata:queuedata,
            }))),
        }
    }
    ///
    /// readlock a 'Cura',returning a guard that can be
    /// dereferenced for read-only operations
    ///
    pub fn read(&self)->ReadGuard<T>
    {
        //TBD think through these memory orderings

        //  how many times have we looped here...
        let mut loops=0;
        loop{
            let lock=self.data().lockcount.fetch_update(
                                        SeqCst,
                                        SeqCst,
                                        |x|{
                                            if x>=0{
                                                Some(x+1)
                                            }else{
                                                None
                                            }
                                        });
            match lock {
                Err(_)=>{/*   its probably writelocked,so we will spin*/
                    if loops>3 || self.queue_size()>0
                    {
                        self.enqueue(LockType::Read);
                        loops=0;
                    }else{
                        loops+=1;
                        std::hint::spin_loop();
                    }
                },
                Ok(_x)=>{/*    x readers,including us*/
                    //  let everyone else in from the queue
                    if self.queue_size()>0
                    {
                        self.wakereader();
                    }
                    break;
                },
            }
        }
        ReadGuard{
            cura:self,
        }
    }
    ///
    /// writelock a 'Cura' , returning a guard that can be
    /// dereferenced for write-operations.
    ///
    pub fn write(&self)->Guard<T>
    {
        //TBD think through these memory orderings
        let mut loops=0;
        loop{
            let lock=self.data().lockcount.fetch_update(
                                        SeqCst,
                                        SeqCst,
                                        |x|{
                                            if x==FREE{
                                                Some(LOCKED)
                                            }else{
                                                None
                                            }
                                        });
            match lock {
                Err(_)=>{/*   its write/readlocked,so we will spin*/
                    if loops>3 || self.queue_size()>0
                    {
                        self.enqueue(LockType::Write);
                        loops=0;
                    }else{
                        loops+=1;
                        std::hint::spin_loop();
                    }
                },
                Ok(_x)=>{/*    should be just us , writing*/
                    break;
                },
            }
        }
        Guard{
            cura:self,
        }
    }
    ///
    /// transparently take a writelock, attempt to mutate the value
    /// and then release the lock
    /// ```
    /// use cura::Cura;
    /// let t=Cura::new(1);
    /// t.alter(|x|{
    ///     if(*x==1){
    ///         Some(2)
    ///     }else{
    ///         None  //dont alter
    ///     }
    ///  });
    ///
    /// ```
    pub fn alter(&self,f:fn(&T)->Option<T>)->Option<()>
    {
        let mut lock=self.write(); //lock
        let v=f(&*lock);
        match v{
            None=>{None},
            Some(tt)=>{
                (*lock)=tt;
                Some(())
            },
        }
    }
}
///
/// cura private stuff
///
impl<T: Sync + Send> Cura<T> {
    ///
    /// util to get accesss to curadata
    ///
    fn data(&self) -> &CuraData<T> {
        unsafe { self.ptr.as_ref() }
    }
    ///
    /// util to get access to the internal queuedata
    ///
    fn get_queuedata(&self) -> *mut QueueData
    {
        self.data().queuedata.get()
    }
    /*
    ///
    /// compare queue count to LOCḰQUEUE to see if it is already
    /// locked
    ///
    fn queue_locked(&self)->bool{
        self.data().queuecount.load(Acquire)>=LOCKQUEUE
    }*/
    ///
    /// spin until we can acquire a lock on queue by incrementing
    /// it with LOCKQUEUE
    ///
    fn lock_queue(&self)
    {
        loop{
            let lock=self.data().queuecount.fetch_update(
                                        SeqCst,
                                        SeqCst,
                                        |x|{
                                            if x<LOCKQUEUE{
                                                Some(x+LOCKQUEUE)
                                            }else{
                                                None
                                            }
                                        });
            match lock {
                Err(_)=>{
                    /*  it is already locked, so we spin*/
                    std::hint::spin_loop();
                },
                Ok(_x)=>{
                    /*  locked successfully*/
                    break;
                },
            }

        }
    }
    ///
    /// check that queue is locked and unlock it by decrementing
    /// by LOCKQUEUE
    ///
    fn unlock_queue(&self)
    {
        let _lock=self.data().queuecount.fetch_update(
                                    SeqCst,
                                    SeqCst,
                                    |x|{
                                        if x<LOCKQUEUE {
                                            panic!("trying to unlock nonlocked queue");
                                        }else{
                                            Some(x-LOCKQUEUE)
                                        }
                                    });
    }
    ///
    /// lock queue and insert ourselves to it and park
    /// waiting for the time in the future when we are
    /// unparked as the first in the queue
    ///
    fn enqueue(&self,t:LockType){

        //  lock and increment queue size
        self.lock_queue();
        self.inc_queue();

        //  insert ourselves into queue
        unsafe{
            (*self.get_queuedata()).enqueue(t);
        }
        //  unlock queue for others to modify and see
        self.unlock_queue();

        //  and park, ready to spin on return
        loop{
            std::thread::park();
            self.lock_queue();
            let amfirst=unsafe{
                    (*(*self.get_queuedata()).queue).thread.id()==std::thread::current().id()
                };
            if amfirst
            {
                unsafe{
                    (*self.get_queuedata()).dequeue();
                }
                self.dec_queue();
                self.unlock_queue();
                break;
            }else{
                self.wakenext();
                self.unlock_queue();
            }
        }
    }
    ///
    /// increment number of threads blocked in queue
    ///
    fn inc_queue(&self)
    {
        self.data().queuecount.fetch_add(
                                    1,
                                    SeqCst);
    }
    ///
    /// decrement number of threads blocked in queue
    ///
    fn dec_queue(&self)
    {
        self.data().queuecount.fetch_sub(1,SeqCst);
    }
    ///
    /// find out approximate size of queue
    ///
    fn queue_size(&self)->u32
    {
        self.data().queuecount.load(Acquire)
    }
    ///
    /// assumes queue is already locked by us 
    ///
    fn wakenext(&self)
    {
        unsafe{
            if !(*self.get_queuedata()).queue.is_null()
            {
                (*(*self.get_queuedata()).queue).thread.unpark();
            }
        }
    }
    ///
    /// wake reader  in front of queue
    ///
    fn wakereader(&self)
    {
        self.lock_queue();
        unsafe{
            let qdata=self.get_queuedata();
            if !(*qdata).queue.is_null()
            {
                if (*(*qdata).queue).lock==LockType::Read
                {
                    (*(*qdata).queue).thread.unpark();
                }
            }
        }
        self.unlock_queue();
    }
    ///
    /// release write lock
    ///
    fn unwritelock(&self)
    {
        self.lock_queue();
        self.wakenext();
        self.unlock_queue();
        let lock=self.data().lockcount.compare_exchange(
                                    LOCKED,FREE,SeqCst,SeqCst);
        match lock {
            Ok(LOCKED) =>{}, //ok
            Ok(x)=>panic!("was supposed to be locked but was {}",x),
            Err(x)=>panic!("was supposed to be locked but was {}",x),
        }
    }
    ///
    /// decrement number of readlocks held
    ///
    fn unreadlock(&self)
    {
        let lock=self.data().lockcount.fetch_sub(1,SeqCst);
        if lock<1
        {
            panic!("was supposed to be readlocked but was {}",1);
        }
        self.lock_queue();
        self.wakenext();
        self.unlock_queue();
    }
}

/**
 *  implement send and sync since thats all we want
 */
unsafe impl<T: Send + Sync> Send for Cura<T> {}
unsafe impl<T: Send + Sync> Sync for Cura<T> {}

/**
 *  deref to make use simpler, this should also transparently
 *  read-lock
 */
//TBD  feed this to chatgpt
/*
impl<T:Sync+Send> Deref for Cura<T>
{
    type Target = ReadGuard<T>;
    //TBD this should probably return a reference to readguard?
    fn deref(&self) -> &Self::Target {
        todo!("this deref should actually do a 'read()' ");
        //&self.data().data
        &self.read()
    }
}*/
/**
 *  clone to make new references of the object
 */
impl<T: Sync + Send> Clone for Cura<T> {
    fn clone(&self) -> Self {
        self.data().count.fetch_add(1, Relaxed);
        Cura { ptr: self.ptr }
    }
}
/**
 *  drop to clean up references
 */
impl<T: Sync + Send> Drop for Cura<T> {
    fn drop(&mut self) {
        if self.data().count.fetch_sub(1, Release) == 1 {
            unsafe {
                drop(Box::from_raw(self.ptr.as_ptr()));
            }
        }
    }
}
/**********************************************************
 *  guards
 */
///
/// writeguard for Cura
///
#[must_use = "if unused the Lock will immediately unlock"]
#[clippy::has_significant_drop]
pub struct Guard<'a,T:Send+Sync>
{
    cura:&'a Cura<T>,
}
impl<T:Send+Sync> Drop for Guard<'_,T>
{
    fn drop(&mut self) {
        self.cura.unwritelock(); //TBD no need to do anything else?
    }
}
impl<T: Sync + Send> Deref for Guard<'_,T> {
    type Target = T;
    fn deref(&self) -> &T { //TBD reference lifetime?
        unsafe{
            &*self.cura.data().data.get()
        }
    }
}
impl<T: Sync + Send> DerefMut for Guard<'_,T> {
    fn deref_mut(&mut self) -> &mut T { //TBD reference lifetime?
        unsafe {
            &mut *self.cura.data().data.get()
        }
    }
}


/**
 *  readguard for Cura
 */
#[must_use = "if unused the Lock will immediately unlock"]
#[clippy::has_significant_drop]
pub struct ReadGuard<'a,T:Send+Sync>
{
    cura:&'a Cura<T>,
}
impl<T:Send+Sync> Drop for ReadGuard<'_,T>
{
    fn drop(&mut self) {
        self.cura.unreadlock(); //TBD nothing else?
    }
}
impl<T: Sync + Send> Deref for ReadGuard<'_,T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe{
            &*self.cura.data().data.get()
        }
    }
}
///
/// util to get current time in millis for testing
///
fn current_time()->u128{
    SystemTime::now().
        duration_since(UNIX_EPOCH).
        expect("weird shit happened").
        as_millis()
}
///
/// util to sĺeep for a few millis
///
fn sleep(millis:u32){
    std::thread::sleep(std::time::Duration::from_millis(millis.into()));
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_usecases() {

        /*  calculator for hits to "testing"*/
        static NUM_HITS: AtomicUsize = AtomicUsize::new(0);
        /*  testing struct to act as the value*/
        struct Foo {
            id:u16,
        }
        impl Foo {
            pub fn new(id:u16) -> Foo {
                Foo {
                    id:id,
                }
            }
            pub fn testing(&self) {
                println!("works {}",self.id);
                NUM_HITS.fetch_add(self.id.into(), SeqCst);
            }
        }
        /*  create ref and hit the test method*/
        let x = Cura::new(Foo::new(1));
        let y = x.clone();
        {
            x.read().testing();
        }
        /*  take a writelock just for fun*/
        {
            let mut w=x.write();
            *w=Foo::new(10);
            w.testing();
        }
        y.read().testing(); //TBD convert this to work with deref
        assert_eq!(21, NUM_HITS.load(Acquire));
    }
    #[test]
    fn advanced_usecases()
    {
        static STATE:AtomicUsize=AtomicUsize::new(0);

        struct Foo{
            id:u16,
        }
        impl Foo {
            pub fn new(id:u16)->Foo{
                Foo{id:id}
            }
            pub fn testing(&mut self,id:u16) {
                self.id=id;
            }
            pub fn id(&self)->u16
            {
                self.id
            }
        }
        /*  create ref*/
        let x=Cura::new(Foo::new(1));
        let y=x.clone();
        /*  writelock one ref*/
        let w=y.write();
        //let start=current_time();
        /*  start creating write and read threads to block*/
        let mut threads=Vec::new();
        for i in 0..30 {
            let c=x.clone();
            let write= i%5==0;
            let i=i;
            let t = std::thread::spawn(move || {
                if write {
                    let c=c.clone();
                    loop{
                        println!("loop {} {}",i,write);
                        let _foo=c.write();
                        STATE.fetch_add(1,SeqCst);
                        //block and loop until STATE
                        if STATE.load(SeqCst)>=(i+1)
                        {
                            break;
                        }
                    }
                }else{
                    let c=c.clone();
                    loop{
                        println!("loop {} {}",i,write);
                        let _foo=c.read();
                        STATE.fetch_add(1,SeqCst);
                        //TBD loop here until STATE>=i
                        if STATE.load(SeqCst)>=(i+1)
                        {
                            break;
                        }
                    }
                }
            });
            threads.push(t);
        }
        sleep(1000);
        drop(w);
        while let Some(t) = threads.pop()
        {
            t.join().unwrap();
            println!("joined");
        }

        //let end=current_time();
        //println!("took:{}",(end-start));
    }
    #[test]
    fn alter_works()
    {
        let t=Cura::new(3);
        t.alter(|x|{
            if *x==2{
                Some(3)
            }else{
                None
            }
        });

        t.alter(|x|{
            if *x==3{
                Some(4)
            }else{
                None
            }
        });

    }
    #[test]
    fn it_works() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct Dropped;

        impl Drop for Dropped {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Relaxed);
            }
        }

        /*  create two testobjects and keep track of dropping*/
        /*  by including the drop-detector into a tuple*/
        let x = Cura::new(("salve!", Dropped));
        let y = x.clone();

        /*  push to another thread, see it works there*/
        let t = std::thread::spawn(move || {
            assert_eq!(x.read().0, "salve!");//TBD conver to deref
        });

        /*  and still works here*/
        assert_eq!(y.read().0, "salve!"); //TBD convert to deref

        /*  wait for the thread*/
        t.join().unwrap();

        /*  object shouldnt have dropped yet*/
        assert_eq!(DROPS.load(Relaxed), 0);

        /*  and we drop the last reference here , so it should drop*/
        drop(y);

        /*  and check the result.*/
        assert_eq!(DROPS.load(Relaxed), 1);
    }
}

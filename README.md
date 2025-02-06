# A sort of an Arc-RwLock combination.. 

That aims to be simple and straightforward to use when sharing data
between threads. 

An attempt at creating an Arc-RwLock combination that is straightforward
to use and no hassle , instead of worrying about being fast and lean. 

 * cloning referefences works like Arc
 * made for sharing objects between threads without worry
 * locking things works like RwLock with write() or read()
 * it spins a few times and then queues if a lock is not obtained
 * miri seems to be happy , so i trust it doesnt leak too much memory etc.
 * requires that everything you stick into it is Send+Sync
 * no need to constantly .unwrap() things instead it will just
   block forever or blow up

# Example
```rust
use cura::Cura;
trait Foo:Send+Sync
{
    fn get(&self)->i32;
    fn set(&mut self,i:i32);
}
#[derive(Debug)]
struct FF{i:i32,};
struct BB{i:i32};

impl Foo for FF{
    fn get(&self)->i32{return self.i;}
    fn set(&mut self,i:i32){self.i=i;}
}

impl Foo for BB{
    fn get(&self)->i32{return self.i;}
    fn set(&mut self,i:i32){self.i=i;}
}

let t=FF{i:1};

// you can do straight "from_box" but currently its impossible to
// "alter" unsized types
let foo2:Cura<dyn Foo>=Cura::from_box(Box::new(FF{i:2}));
let foo:Cura<Box<dyn Foo>>=Cura::new(Box::new(t));
let a=foo.clone();
let b=foo.clone();

{
    assert_eq!(a.read().get(),1);
    {
        a.alter(|s|{
            s.set(2);
            Some(())
        });
    }
    {
        a.alter(|s|{ //this only works for Sized types
            *s=Box::new(BB{i:2});
            Some(())
        });
    }
    let lock=a.read();
    let v=lock;
    assert_eq!(v.get(),2)
}//lock dropped here
{
    (*b.write()).set(3); //lock dropped here i think 
}

assert_eq!((*a.read()).get(),3);

```

## fixing this would improve usability 
https://github.com/rust-lang/rust/issues/18598

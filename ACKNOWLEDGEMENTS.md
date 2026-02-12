# Acknowledgements

## Helps and Contributions

**[Keggek](https://codeberg.org/gek)** - For the discussions about the project and language design <3  
**[Lekebabiste](https://github.com/Lekebabiste)** - For helping with the UDP implementation <3  
**[SpaceGame](https://github.com/SpaceGame-wq)** - For making an Aelys syntax highlighting [VSCode extension](https://marketplace.visualstudio.com/items?itemName=SpaceGame.aelys-lang) <3

## Inspirations

**Rust** - The syntax style, `let mut` for mutability, range expressions (`..` and `..=`).  
Also happens to be what Aelys is written in lol

**Go** - Automatic semicolon insertion, the philosophy of simplicity, fast compilation.

**Lua** - Lightweight VM design, embeddability goals. The original inspiration for trying to build something small and fast.

**Python** - Readability focus. The `and`/`or`/`not` keywords.

Honestly I really want do make a language that feels like a blend of all these things, taking the best ideas from each.  
Some sort of « python but that treats you as an adult »

---

## Usage of AI

Debugging sessions, architectural discussions, and keeping me sane when the VM decides to just.. not work.  

AI also wrote most of the tests for Aelys, some stuff in the examples/ folder (notably the benchmarks), and also some parts of the stdlib, which saved a lot of time.  
I prefer not to focus too much on that and instead work on the VM rather than anything else


Without AI assistance this would've taken 10x longer, maybe more.

This whole thing started as a way to actually understand how compilers work beyond just reading about them, and turns out building one is the best way to learn

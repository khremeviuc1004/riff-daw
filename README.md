# riff-daw

[![Github All Releases](https://img.shields.io/github/downloads/khremeviuc1004/riff-daw/total.svg)]()


A digial audio work station for Linux that uses riffs as building blocks for riff sets, riff sequences and riff arrangements. Each of these can be auditioned independently and can be composed of the preceeding items: a riff arrangement is made up of riff sequences and riff sets and a riff sequence is made up of riff sets.

Build a release version in Linux with:
```bash
RUSTFLAGS="-C target-cpu=native" cargo build --bin riff-daw --release
```

When the UI is slow to draw on Debian Bookworm run with:
```bash
GDK_RENDERING=image VST_PATH=/home/kevin/Desktop/Linux_VST/ CLAP_PATH=/home/kevin/Desktop/Linux_VST/ ./target/release/riff-daw
```

To check thread performance on Linux using htop adapted from [unix.stackexchange.com](https://unix.stackexchange.com/questions/183514/how-to-display-only-a-process-and-its-descendant-processes-on-htop):
```bash
ps -ef | grep riff-daw
htop -p $(pstree -p <process id> | perl -ne 'push @t, /\((\d+)\)/g; END { print join ",", @t }')
```
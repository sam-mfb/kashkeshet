# Name packs

Each `.txt` file here becomes a pack built into the binary. Its filename
without the extension is its lowercase ASCII name. Add a file and rebuild;
no Rust changes or runtime files are needed.

Start with `[first]`, then whitespace-separated first words, then a line
`[second]`, then whitespace-separated second words. Each list must contain
more than 36 unique words, all lowercase ASCII letters. Use short, plain,
inoffensive words. Avoid commands and roles. The build checks the format,
letters, length and duplicates; tests check the same rules for every pack.

The `animals` pack is the default and retains every original word. It has
over 200 words per list. `garden` pairs descriptive words with plants.
Set `name_pack = "garden"` in the server config or pass
`kkt serve --name-pack garden`. Only new registrations use the chosen pack;
existing names and the shared server name uniqueness rules stay the same.

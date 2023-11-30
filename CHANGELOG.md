# Changelog

## 2.0.0 (2023-11-30)


### Features

* allow requesting songs by url with !p ([6be6eb7](https://github.com/FruitieX/irc-sitz-rs/commit/6be6eb782d0b83b5564b2aff9e4ee591369a0a98))
* basic irc functionality ([14eb006](https://github.com/FruitieX/irc-sitz-rs/commit/14eb006039380f8c8f36b3be91404570c184d4ff))
* duration check ([0dd5926](https://github.com/FruitieX/irc-sitz-rs/commit/0dd592658ff0888bfe60a9f29864105e7454e5b4))
* emit eof events when playback finishes ([8dc5de5](https://github.com/FruitieX/irc-sitz-rs/commit/8dc5de5f592b2f5cafdc64d76beee85645de7c88))
* espeak source ([b33ba49](https://github.com/FruitieX/irc-sitz-rs/commit/b33ba49e74cf07d22ac32f204ecf00b56b25088d))
* fetch song metadata from songbook url ([a3555d8](https://github.com/FruitieX/irc-sitz-rs/commit/a3555d84001e24ac22961f2010a98a8d0308d000))
* force request command ([a0530ad](https://github.com/FruitieX/irc-sitz-rs/commit/a0530addb286967d6c9fb6d58603b1e349293426))
* generate espeak audio samples outside audio task ([e70de67](https://github.com/FruitieX/irc-sitz-rs/commit/e70de67f8a5002fbf6a41035767f63c440a16d92))
* improve ListSongs formatting ([baf8ff1](https://github.com/FruitieX/irc-sitz-rs/commit/baf8ff1eb0d35400b281fbe48e8bbae7ebac38b6))
* list queue length ([0720e52](https://github.com/FruitieX/irc-sitz-rs/commit/0720e52aa8e81ced518271d27d3d137c9453f5a6))
* logging levels ([2eee12c](https://github.com/FruitieX/irc-sitz-rs/commit/2eee12c2e4a4bd0f8487eb9442aed75e19d83a4b))
* music playback can start when program starts ([411dce5](https://github.com/FruitieX/irc-sitz-rs/commit/411dce56a98c8fa1c4eba42d6b98c85905f407d9))
* music playback queue ([f3f3120](https://github.com/FruitieX/irc-sitz-rs/commit/f3f3120a22da57954ba94406bbc55288f0955b2d))
* music volume command ([9036599](https://github.com/FruitieX/irc-sitz-rs/commit/90365995372de92967ada0f56589b810dfd67aaf))
* nodelay and http support ([51cbe07](https://github.com/FruitieX/irc-sitz-rs/commit/51cbe0704a4cfa6b08f6e2ea90fb882eeb92c3e9))
* pass samples through tokio watch channel ([fd3f7f9](https://github.com/FruitieX/irc-sitz-rs/commit/fd3f7f9e6aacdba2a17911d8fd8b25812cd67810))
* pause playback on errors ([cb921c6](https://github.com/FruitieX/irc-sitz-rs/commit/cb921c690661e8667210b42ab30a3ac17a6ab889))
* playback via symphonia ([65295dd](https://github.com/FruitieX/irc-sitz-rs/commit/65295dd831cb27cb0c90550083c743aa55c5fd87))
* playback via yt-dlp ([6184bf7](https://github.com/FruitieX/irc-sitz-rs/commit/6184bf7e1fa5c662e7422d376b265d4db3723837))
* prevent duplicate requests ([898f160](https://github.com/FruitieX/irc-sitz-rs/commit/898f16091184936292597d555dc054a4d0461b15))
* prevent music queue duplicates ([6ec3086](https://github.com/FruitieX/irc-sitz-rs/commit/6ec3086dc4515a6037386578577d189d205d7f52))
* print version number when starting ([f987721](https://github.com/FruitieX/irc-sitz-rs/commit/f98772142d51c4fe4821ded9b5bef1d084e70be1))
* remove most recently queued song by nick ([4f73001](https://github.com/FruitieX/irc-sitz-rs/commit/4f73001619cb0125473f54558c94fef0dadc419b))
* secondary mixer channel "ducking" ([a39d831](https://github.com/FruitieX/irc-sitz-rs/commit/a39d8318d71f2937e732af7d3c931d6dc773a7a1))
* shorten song request list output format ([a00765c](https://github.com/FruitieX/irc-sitz-rs/commit/a00765c5e3ebc23fbe0489174877a12c323a2154))
* songleader implementation ([fc4ecb6](https://github.com/FruitieX/irc-sitz-rs/commit/fc4ecb6bebacbda83ebb53b52532118910d95627))
* support listing songs by offset, removing ([065d436](https://github.com/FruitieX/irc-sitz-rs/commit/065d436c6a410386780a7ed71c7252fb56874925))
* support removing latest song by other nick ([4a39004](https://github.com/FruitieX/irc-sitz-rs/commit/4a390047c71b040e55e4544a606bc3ad1947e0a1))
* trigger tts with events ([78a8c5a](https://github.com/FruitieX/irc-sitz-rs/commit/78a8c5a6204585ef23f9da710bfd9029c2af889b))


### Bug Fixes

* avoid panicking on errors ([ea9c69d](https://github.com/FruitieX/irc-sitz-rs/commit/ea9c69d2b63914c1222fefd13333ec85b9701652))
* cancel decoding on incoming decode task ([baa10f5](https://github.com/FruitieX/irc-sitz-rs/commit/baa10f5bb6199776c0ae3cdca5e6c752fae4e2e9))
* clear pause flag on playback actions ([ff0123b](https://github.com/FruitieX/irc-sitz-rs/commit/ff0123b016a33197e3febca690769470c851682e))
* **deps:** pin dependencies ([814ce56](https://github.com/FruitieX/irc-sitz-rs/commit/814ce5661045e4ba766c21fc051897c39ae4c53c))
* **deps:** update rust crate itertools to v0.12.0 ([734346b](https://github.com/FruitieX/irc-sitz-rs/commit/734346b940a85246fe6e475856616d5284eb200c))
* **deps:** update rust crate serde to v1.0.193 ([8f7d664](https://github.com/FruitieX/irc-sitz-rs/commit/8f7d6644e43cfd90dc8f033bef081d684d8cacb3))
* **deps:** update rust crate tokio to v1.34.0 ([fe34056](https://github.com/FruitieX/irc-sitz-rs/commit/fe34056df18c8c3509c33cf08c13019c7947ab8c))
* don't download entire playlist ([e6a5b85](https://github.com/FruitieX/irc-sitz-rs/commit/e6a5b854a46fa0818dfa926f00abc17a9d06e267))
* don't segfault if calling espeak too rapidly ([ecb3dd4](https://github.com/FruitieX/irc-sitz-rs/commit/ecb3dd49d35468a9b6072fbbba6ca93adde12131))
* instruct user to use shorter bingo command ([08a1562](https://github.com/FruitieX/irc-sitz-rs/commit/08a1562f78e498f4c756b70d5b9245d695e86fd6))
* non blocking stdin handling ([39c0242](https://github.com/FruitieX/irc-sitz-rs/commit/39c0242f87705dbed16deb2d1f3dfee4d2a9089a))
* persist state in more cases ([3273b05](https://github.com/FruitieX/irc-sitz-rs/commit/3273b050314fcadb992cecc461d163f3cbd51b81))
* reduce tempo & bingo nick count, fix off by one ([7907587](https://github.com/FruitieX/irc-sitz-rs/commit/7907587b071643437d8148edf99e62ebcb3bdc59))
* skip to next song if removing current song ([2ec9027](https://github.com/FruitieX/irc-sitz-rs/commit/2ec9027cae418437aa8c9b7b69788336dc8911c8))
* tweaks to songleader timings, ForceSinging action ([4a28bce](https://github.com/FruitieX/irc-sitz-rs/commit/4a28bce07673ba4e131d5b86f8d0bc10ecd426ce))


### Miscellaneous Chores

* release 2.0.0 ([7efb69b](https://github.com/FruitieX/irc-sitz-rs/commit/7efb69b09daecd184a1f660788807b81f8a43b39))

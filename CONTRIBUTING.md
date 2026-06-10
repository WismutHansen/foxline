# Contribution

This is a hobby project but I am happy to accept contributions as long as they adhere to the following guidelines:

- We can't ship any actual game assets in the repo so anything we use must be programmatically extractable from the source media by the users.
- I'm building this around pi-agent because this is my AI agent harness of choice. If you want to add support for other harnesses, this should be via optional interfaces users can chose from

High value contribution ideas:

- Figuring out how to provide higher quality source files especially for the reference voices to make the cloned voices sound less robotic. Maybe just selecting better snippets from the source files as reference can already improve the output.
- Testing out different text-to-speech and speech-to-text models and benchmarking them against the current defaults
- Adding support for Linux and Windows and testing on different platforms
- Creating workflows so users can generate their own characters adhering to the overall style. For example, training a lora for one of the sota open source text-to-image/image-to-image models and creating scripts to automate the asset creation
- Adding more codec styles and characters from the other games

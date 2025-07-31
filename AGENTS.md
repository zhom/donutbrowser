# Instructions for AI Agents

- After your changes, instead of running specific tests or linting specific files, run "pnpm format && pnpm lint && pnpm test". It means that you first format the code, then lint it, then test it, so that no part is broken after your changes.
- Don't leave comments that don't add value.
- Do not duplicate code unless you have a very good reason to do so. It is important that the same logic is not duplicated multiple times.
- Before finishing the task and showing summary, always run "pnpm format && pnpm lint && pnpm test" at the root of the project to ensure that you don't finish with broken application.
- Anytime you change nodecar's code and try to test, recompile it with "cd nodecar && pnpm build".
- If there is a global singleton of a struct, only use it inside a method while properly initializing it, unless I have explicitly specified in the request otherwise.

# Project Guidelines

## Testing and Quality

- After making changes, run `pnpm format && pnpm lint && pnpm test` at the root of the project
- Always run this command before finishing a task to ensure the application isn't broken

## Code Quality

- Don't leave comments that don't add value
- Don't duplicate code unless there's a very good reason; keep the same logic in one place

## Nodecar

- After changing nodecar's code, recompile it with `cd nodecar && pnpm build` before testing

## Singletons

- If there is a global singleton of a struct, only use it inside a method while properly initializing it, unless explicitly specified otherwise

## UI Theming

- When modifying the UI, don't add random colors that are not controlled by `src/lib/themes.ts`

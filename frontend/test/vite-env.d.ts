// Vitest loads these through Vite. The app tsconfig excludes test/, so the
// typechecker needs the two Vite features the harness uses.
interface ImportMeta {
  glob(
    pattern: string,
    options?: { query?: string; import?: string; eager?: boolean },
  ): Record<string, string>;
}

declare module '*?raw' {
  const content: string;
  export default content;
}

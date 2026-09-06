import type { CSSProperties } from 'vue';

export { default as JsonView, default as default } from './json-view.vue';
export type { JsonViewProps } from './json-view.vue';

export const githubLightTheme: CSSProperties = {
    color: '#24292f',
    backgroundColor: 'transparent',
};

export const githubDarkTheme: CSSProperties = {
    color: '#c9d1d9',
    backgroundColor: 'transparent',
};

export { default as Calendar } from './calendar.vue';
export { default } from './calendar.vue';

export type {
  CalendarProps,
  CalendarClassNames,
  CalendarDisabledMatcher,
  CalendarDayInfo,
} from './calendar.vue';

// 兼容 React 版 react-day-picker 导出的 DayButton 与 CalendarDayButton 概念
export { default as CalendarDayButton } from './calendar.vue';

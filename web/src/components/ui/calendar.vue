<script setup lang="ts">
import { computed, ref, watch, type HTMLAttributes } from 'vue';
import dayjs, { type Dayjs } from 'dayjs';
import { LuChevronLeft, LuChevronRight, LuChevronDown } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';

export type CalendarDisabledMatcher =
  | boolean
  | ((date: Date) => boolean)
  | { before?: Date; after?: Date }
  | Date[];

export interface CalendarClassNames {
  root?: string;
  months?: string;
  month?: string;
  nav?: string;
  button_previous?: string;
  button_next?: string;
  month_caption?: string;
  dropdowns?: string;
  dropdown_root?: string;
  dropdown?: string;
  caption_label?: string;
  month_grid?: string;
  weekdays?: string;
  weekday?: string;
  week?: string;
  week_number_header?: string;
  week_number?: string;
  day?: string;
  day_button?: string;
  range_start?: string;
  range_middle?: string;
  range_end?: string;
  today?: string;
  outside?: string;
  disabled?: string;
  hidden?: string;
}

export interface CalendarProps {
  modelValue?: Date | string | number | null;
  selected?: Date | string | number | null;
  mode?: 'single' | 'range' | 'multiple';
  disabled?: CalendarDisabledMatcher;
  defaultMonth?: Date | string | Dayjs;
  month?: Date | string | Dayjs;
  showOutsideDays?: boolean;
  captionLayout?: 'label' | 'dropdown';
  buttonVariant?: 'default' | 'destructive' | 'outline' | 'secondary' | 'ghost' | 'link';
  class?: HTMLAttributes['class'];
  className?: string;
  classNames?: Partial<CalendarClassNames>;
  formatters?: {
    formatMonthDropdown?: (date: Date) => string;
    formatCaption?: (date: Date) => string;
    formatWeekdayName?: (dayIndex: number) => string;
  };
  fixedWeeks?: boolean;
}

const props = withDefaults(defineProps<CalendarProps>(), {
  modelValue: undefined,
  selected: undefined,
  mode: 'single',
  disabled: false,
  defaultMonth: undefined,
  month: undefined,
  showOutsideDays: true,
  captionLayout: 'label',
  buttonVariant: 'ghost',
  fixedWeeks: false,
});

const emit = defineEmits<{
  (e: 'update:modelValue', value: Date | undefined): void;
  (e: 'update:selected', value: Date | undefined): void;
  (e: 'select', value: Date | undefined): void;
  (e: 'update:month', value: Date): void;
  (e: 'monthChange', value: Date): void;
}>();

const selectedDate = computed<Dayjs | null>(() => {
  const val = props.selected !== undefined ? props.selected : props.modelValue;
  if (!val) return null;
  const d = dayjs(val);
  return d.isValid() ? d : null;
});

const initialMonth = props.month || props.defaultMonth || props.selected || props.modelValue || new Date();
const internalMonth = ref<Dayjs>(dayjs(initialMonth).startOf('month'));

watch(
  () => props.month,
  (newMonth) => {
    if (newMonth) {
      const d = dayjs(newMonth);
      if (d.isValid()) {
        internalMonth.value = d.startOf('month');
      }
    }
  }
);

const currentMonth = computed(() => internalMonth.value);

const handlePrevMonth = () => {
  const nextMonth = internalMonth.value.subtract(1, 'month');
  internalMonth.value = nextMonth;
  emit('update:month', nextMonth.toDate());
  emit('monthChange', nextMonth.toDate());
};

const handleNextMonth = () => {
  const nextMonth = internalMonth.value.add(1, 'month');
  internalMonth.value = nextMonth;
  emit('update:month', nextMonth.toDate());
  emit('monthChange', nextMonth.toDate());
};

const captionText = computed(() => {
  const date = currentMonth.value.toDate();
  if (props.formatters?.formatCaption) {
    return props.formatters.formatCaption(date);
  }
  return date.toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
});

const weekdays = computed(() => {
  const baseSunday = dayjs().startOf('week'); // Sunday
  const list: string[] = [];
  for (let i = 0; i < 7; i++) {
    if (props.formatters?.formatWeekdayName) {
      list.push(props.formatters.formatWeekdayName(i));
    } else {
      const dayName = baseSunday.add(i, 'day').toDate().toLocaleDateString(undefined, { weekday: 'narrow' });
      list.push(dayName || ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'][i]);
    }
  }
  return list;
});

const isDateDisabled = (d: Dayjs): boolean => {
  if (props.disabled === true) return true;
  if (typeof props.disabled === 'function') {
    return props.disabled(d.toDate());
  }
  if (Array.isArray(props.disabled)) {
    return props.disabled.some((item) => dayjs(item).isSame(d, 'day'));
  }
  if (typeof props.disabled === 'object' && props.disabled !== null) {
    const { before, after } = props.disabled;
    if (before && d.isBefore(dayjs(before), 'day')) return true;
    if (after && d.isAfter(dayjs(after), 'day')) return true;
  }
  return false;
};

export interface CalendarDayInfo {
  date: Date;
  dayjsDate: Dayjs;
  dayNumber: number;
  isCurrentMonth: boolean;
  isToday: boolean;
  isSelected: boolean;
  isDisabled: boolean;
  isOutside: boolean;
}

const weeks = computed<CalendarDayInfo[][]>(() => {
  const monthStart = currentMonth.value.startOf('month');
  const monthEnd = currentMonth.value.endOf('month');
  const startDayOfWeek = monthStart.day(); // 0 = Sun
  const startDate = monthStart.subtract(startDayOfWeek, 'day');

  const totalDays = startDayOfWeek + monthEnd.date();
  const numWeeks = props.fixedWeeks ? 6 : Math.ceil(totalDays / 7);
  const today = dayjs();

  const result: CalendarDayInfo[][] = [];
  for (let w = 0; w < numWeeks; w++) {
    const weekDays: CalendarDayInfo[] = [];
    for (let d = 0; d < 7; d++) {
      const dayDate = startDate.add(w * 7 + d, 'day');
      const isCurrent = dayDate.month() === currentMonth.value.month();
      const isToday = dayDate.isSame(today, 'day');
      const isSelected = selectedDate.value ? dayDate.isSame(selectedDate.value, 'day') : false;
      const isDisabled = isDateDisabled(dayDate);

      weekDays.push({
        date: dayDate.toDate(),
        dayjsDate: dayDate,
        dayNumber: dayDate.date(),
        isCurrentMonth: isCurrent,
        isToday,
        isSelected,
        isDisabled,
        isOutside: !isCurrent,
      });
    }
    result.push(weekDays);
  }
  return result;
});

const handleSelect = (day: CalendarDayInfo) => {
  if (day.isDisabled) return;

  const newDate = day.isSelected ? undefined : day.date;
  emit('update:modelValue', newDate);
  emit('update:selected', newDate);
  emit('select', newDate);

  if (day.isOutside) {
    internalMonth.value = day.dayjsDate.startOf('month');
    emit('update:month', internalMonth.value.toDate());
    emit('monthChange', internalMonth.value.toDate());
  }
};

const defaultClassNames: Record<keyof CalendarClassNames, string> = {
  root: 'w-fit',
  months: 'relative flex flex-col gap-4 md:flex-row',
  month: 'flex w-full flex-col gap-4',
  nav: 'absolute inset-x-0 top-0 flex w-full items-center justify-between gap-1',
  button_previous:
    'size-8 p-0 select-none aria-disabled:opacity-50 inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50',
  button_next:
    'size-8 p-0 select-none aria-disabled:opacity-50 inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50',
  month_caption: 'flex h-8 w-full items-center justify-center px-8',
  dropdowns: 'flex h-8 w-full items-center justify-center gap-1.5 text-sm font-medium',
  dropdown_root:
    'relative rounded-md border border-input shadow-xs has-focus:border-ring has-focus:ring-[3px] has-focus:ring-ring/50',
  dropdown: 'absolute inset-0 bg-popover opacity-0',
  caption_label: 'font-medium select-none text-sm',
  month_grid: 'w-full border-collapse',
  weekdays: 'flex',
  weekday: 'flex-1 rounded-md text-[0.8rem] font-normal text-muted-foreground select-none text-center',
  week: 'mt-2 flex w-full',
  week_number_header: 'w-8 select-none',
  week_number: 'text-[0.8rem] text-muted-foreground select-none',
  day: 'group/day relative aspect-square h-full w-full p-0 text-center select-none [&:last-child[data-selected=true]_button]:rounded-r-md [&:first-child[data-selected=true]_button]:rounded-l-md',
  day_button:
    'flex aspect-square size-auto w-full min-w-8 flex-col gap-1 leading-none font-normal group-data-[focused=true]/day:relative group-data-[focused=true]/day:z-10 group-data-[focused=true]/day:border-ring group-data-[focused=true]/day:ring-[3px] group-data-[focused=true]/day:ring-ring/50 data-[range-end=true]:rounded-md data-[range-end=true]:rounded-r-md data-[range-end=true]:bg-primary data-[range-end=true]:text-primary-foreground data-[range-middle=true]:rounded-none data-[range-middle=true]:bg-accent data-[range-middle=true]:text-accent-foreground data-[range-start=true]:rounded-md data-[range-start=true]:rounded-l-md data-[range-start=true]:bg-primary data-[range-start=true]:text-primary-foreground data-[selected-single=true]:bg-primary data-[selected-single=true]:text-primary-foreground dark:hover:text-accent-foreground [&>span]:text-xs [&>span]:opacity-70 inline-flex items-center justify-center rounded-md text-sm hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50',
  range_start: 'rounded-l-md bg-accent',
  range_middle: 'rounded-none',
  range_end: 'rounded-r-md bg-accent',
  today: 'rounded-md bg-accent text-accent-foreground data-[selected=true]:rounded-none',
  outside: 'text-muted-foreground opacity-50',
  disabled: 'text-muted-foreground opacity-50 pointer-events-none',
  hidden: 'invisible',
};
</script>

<template>
  <div
    data-slot="calendar"
    :class="
      cn(
        'group/calendar bg-background p-3 [--cell-size:--spacing(8)] [[data-slot=card-content]_&]:bg-transparent [[data-slot=popover-content]_&]:bg-transparent',
        'rtl:**:[.rdp-button\\_next>svg]:rotate-180',
        'rtl:**:[.rdp-button\\_previous>svg]:rotate-180',
        defaultClassNames.root,
        props.classNames?.root,
        props.class,
        props.className
      )
    "
  >
    <div :class="cn(defaultClassNames.months, props.classNames?.months)">
      <div :class="cn(defaultClassNames.month, props.classNames?.month)">
        <!-- Navigation -->
        <div :class="cn(defaultClassNames.nav, props.classNames?.nav)">
          <button
            type="button"
            aria-label="Previous month"
            :class="cn(defaultClassNames.button_previous, props.classNames?.button_previous)"
            @click="handlePrevMonth"
          >
            <slot name="previous-icon">
              <LuChevronLeft class="size-4" />
            </slot>
          </button>
          <button
            type="button"
            aria-label="Next month"
            :class="cn(defaultClassNames.button_next, props.classNames?.button_next)"
            @click="handleNextMonth"
          >
            <slot name="next-icon">
              <LuChevronRight class="size-4" />
            </slot>
          </button>
        </div>

        <!-- Month caption -->
        <div :class="cn(defaultClassNames.month_caption, props.classNames?.month_caption)">
          <slot name="caption" :date="currentMonth.toDate()" :caption="captionText">
            <span :class="cn(defaultClassNames.caption_label, props.classNames?.caption_label)">
              {{ captionText }}
            </span>
          </slot>
        </div>

        <!-- Calendar table / grid -->
        <table :class="cn(defaultClassNames.month_grid, props.classNames?.month_grid)">
          <thead>
            <tr :class="cn(defaultClassNames.weekdays, props.classNames?.weekdays)">
              <th
                v-for="(wName, idx) in weekdays"
                :key="idx"
                scope="col"
                :class="cn(defaultClassNames.weekday, props.classNames?.weekday)"
              >
                {{ wName }}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(week, wIdx) in weeks"
              :key="wIdx"
              :class="cn(defaultClassNames.week, props.classNames?.week)"
            >
              <td
                v-for="(day, dIdx) in week"
                :key="dIdx"
                :class="
                  cn(
                    defaultClassNames.day,
                    day.isOutside && !showOutsideDays && defaultClassNames.hidden,
                    props.classNames?.day
                  )
                "
                :data-selected="day.isSelected"
              >
                <button
                  v-if="showOutsideDays || !day.isOutside"
                  type="button"
                  :disabled="day.isDisabled"
                  :data-day="day.date.toLocaleDateString()"
                  :data-selected-single="day.isSelected"
                  :data-selected="day.isSelected"
                  :class="
                    cn(
                      defaultClassNames.day_button,
                      day.isToday && cn(defaultClassNames.today, props.classNames?.today),
                      day.isOutside && cn(defaultClassNames.outside, props.classNames?.outside),
                      day.isDisabled && cn(defaultClassNames.disabled, props.classNames?.disabled),
                      props.classNames?.day_button
                    )
                  "
                  @click="handleSelect(day)"
                >
                  <slot name="day" :day="day">
                    <span>{{ day.dayNumber }}</span>
                  </slot>
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>

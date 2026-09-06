<script setup lang="ts">
import { computed, inject, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { TABS_INJECTION_KEY } from "./tabs"

interface Props {
  value: string | number
  disabled?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
})

const tabsContext = inject(TABS_INJECTION_KEY, null)

const isActive = computed(() => {
  return tabsContext ? tabsContext.currentValue.value === props.value : false
})

function handleClick() {
  if (props.disabled || !tabsContext) return
  tabsContext.setValue(props.value)
}
</script>

<template>
  <button
    type="button"
    role="tab"
    data-slot="tabs-trigger"
    :data-state="isActive ? 'active' : 'inactive'"
    :aria-selected="isActive ? 'true' : 'false'"
    :disabled="disabled"
    :tabindex="isActive ? 0 : -1"
    :class="
      cn(
        'relative inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-sm font-medium whitespace-nowrap text-foreground/60 transition-all group-data-[orientation=vertical]/tabs:w-full group-data-[orientation=vertical]/tabs:justify-start hover:text-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-1 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 group-data-[variant=default]/tabs-list:data-[state=active]:shadow-sm group-data-[variant=line]/tabs-list:data-[state=active]:shadow-none dark:text-muted-foreground dark:hover:text-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*=\'size-\'])]:size-4',
        'group-data-[variant=line]/tabs-list:bg-transparent group-data-[variant=line]/tabs-list:data-[state=active]:bg-transparent dark:group-data-[variant=line]/tabs-list:data-[state=active]:border-transparent dark:group-data-[variant=line]/tabs-list:data-[state=active]:bg-transparent',
        'data-[state=active]:text-foreground group-data-[variant=default]/tabs-list:data-[state=active]:bg-background dark:group-data-[variant=default]/tabs-list:data-[state=active]:border-input dark:group-data-[variant=default]/tabs-list:data-[state=active]:bg-input/30',
        'after:absolute after:bg-foreground after:opacity-0 after:transition-opacity group-data-[orientation=horizontal]/tabs:after:inset-x-0 group-data-[orientation=horizontal]/tabs:after:bottom-[-5px] group-data-[orientation=horizontal]/tabs:after:h-0.5 group-data-[orientation=vertical]/tabs:after:inset-y-0 group-data-[orientation=vertical]/tabs:after:-right-1 group-data-[orientation=vertical]/tabs:after:w-0.5 group-data-[variant=line]/tabs-list:data-[state=active]:after:opacity-100',
        props.class,
        props.className
      )
    "
    @click="handleClick"
  >
    <slot />
  </button>
</template>

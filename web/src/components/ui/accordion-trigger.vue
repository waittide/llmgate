<script setup lang="ts">
import { computed, inject, type HTMLAttributes } from "vue"
import { LuChevronDown } from "vue-icons-plus/lu"
import { cn } from "@/lib/utils"
import { ACCORDION_ITEM_INJECTION_KEY } from "./accordion"

interface Props {
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
  disabled?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
})

const item = inject(ACCORDION_ITEM_INJECTION_KEY, null)

const isOpen = computed(() => item?.isOpen.value ?? false)
const isDisabled = computed(() => props.disabled || (item?.disabled.value ?? false))

function handleClick() {
  if (isDisabled.value || !item) return
  item.toggle()
}
</script>

<template>
  <div data-slot="accordion-header" class="flex">
    <button
      type="button"
      data-slot="accordion-trigger"
      :data-state="isOpen ? 'open' : 'closed'"
      :aria-expanded="isOpen"
      :disabled="isDisabled"
      :class="
        cn(
          'focus-visible:border-ring focus-visible:ring-ring/50 flex flex-1 items-start justify-between gap-4 rounded-md py-4 text-left text-sm font-medium transition-all outline-none hover:underline focus-visible:ring-[3px] disabled:pointer-events-none disabled:opacity-50 [&[data-state=open]>svg]:rotate-180',
          props.class,
          props.className
        )
      "
      @click="handleClick"
    >
      <slot />
      <LuChevronDown
        class="text-muted-foreground pointer-events-none size-4 shrink-0 translate-y-0.5 transition-transform duration-200"
      />
    </button>
  </div>
</template>

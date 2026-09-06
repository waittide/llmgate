<script setup lang="ts">
import { computed, inject, provide, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import {
  ACCORDION_INJECTION_KEY,
  ACCORDION_ITEM_INJECTION_KEY,
} from "./accordion"

interface Props {
  value: string
  disabled?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  disabled: false,
})

const accordion = inject(ACCORDION_INJECTION_KEY, null)

const isOpen = computed(() => {
  return accordion ? accordion.isOpen(props.value) : false
})

const isDisabled = computed(() => {
  return Boolean(props.disabled || accordion?.disabled.value)
})

function toggle() {
  if (isDisabled.value || !accordion) return
  accordion.toggleItem(props.value)
}

provide(ACCORDION_ITEM_INJECTION_KEY, {
  value: props.value,
  isOpen,
  disabled: isDisabled,
  toggle,
})
</script>

<template>
  <div
    data-slot="accordion-item"
    :data-state="isOpen ? 'open' : 'closed'"
    :data-disabled="isDisabled ? 'true' : undefined"
    :class="cn('border-b last:border-b-0', props.class, props.className)"
  >
    <slot />
  </div>
</template>

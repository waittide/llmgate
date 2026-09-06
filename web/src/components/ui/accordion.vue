<script setup lang="ts">
import { computed, provide, ref, watch, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { ACCORDION_INJECTION_KEY } from "./accordion"

type AccordionType = "single" | "multiple"

const modelValue = defineModel<string | string[]>()

interface Props {
  type?: AccordionType
  collapsible?: boolean
  defaultValue?: string | string[]
  value?: string | string[]
  disabled?: boolean
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  type: "single",
  collapsible: false,
  disabled: false,
})

const emit = defineEmits<{
  "update:value": [value: string | string[]]
  valueChange: [value: string | string[]]
  onValueChange: [value: string | string[]]
}>()

function normalizeValues(v: unknown): string[] {
  if (Array.isArray(v)) return v.map(String)
  if (typeof v === "string" && v) return [v]
  return []
}

const currentValues = ref<string[]>(
  normalizeValues(modelValue.value ?? props.value ?? props.defaultValue)
)

watch(
  () => modelValue.value,
  (val) => {
    if (val !== undefined) {
      currentValues.value = normalizeValues(val)
    }
  }
)

watch(
  () => props.value,
  (val) => {
    if (val !== undefined) {
      currentValues.value = normalizeValues(val)
    }
  }
)

function isOpen(val: string): boolean {
  return currentValues.value.includes(val)
}

function toggleItem(val: string) {
  if (props.disabled) return

  let nextValues: string[]
  if (props.type === "multiple") {
    if (isOpen(val)) {
      nextValues = currentValues.value.filter((v) => v !== val)
    } else {
      nextValues = [...currentValues.value, val]
    }
  } else {
    // single
    if (isOpen(val)) {
      nextValues = props.collapsible ? [] : [val]
    } else {
      nextValues = [val]
    }
  }

  currentValues.value = nextValues
  const emittedValue =
    props.type === "multiple" ? nextValues : (nextValues[0] ?? "")
  modelValue.value = emittedValue
  emit("update:value", emittedValue)
  emit("valueChange", emittedValue)
  emit("onValueChange", emittedValue)
}

provide(ACCORDION_INJECTION_KEY, {
  isOpen,
  toggleItem,
  disabled: computed(() => props.disabled),
})
</script>

<template>
  <div
    data-slot="accordion"
    :class="cn('w-full', props.class, props.className)"
  >
    <slot />
  </div>
</template>

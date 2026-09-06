<script setup lang="ts">
import { computed, provide, ref, watch, type HTMLAttributes } from "vue"
import { cn } from "@/lib/utils"
import { TABS_INJECTION_KEY } from "./tabs"

const modelValue = defineModel<string | number>()

interface Props {
  defaultValue?: string | number
  value?: string | number
  orientation?: "horizontal" | "vertical"
  class?: HTMLAttributes["class"]
  className?: HTMLAttributes["class"]
}

const props = withDefaults(defineProps<Props>(), {
  orientation: "horizontal",
})

const emit = defineEmits<{
  "update:value": [value: string | number]
  valueChange: [value: string | number]
  onValueChange: [value: string | number]
}>()

const currentValue = ref<string | number>(
  modelValue.value ?? props.value ?? props.defaultValue ?? ""
)

watch(
  () => modelValue.value,
  (val) => {
    if (val !== undefined) {
      currentValue.value = val
    }
  }
)

watch(
  () => props.value,
  (val) => {
    if (val !== undefined) {
      currentValue.value = val
    }
  }
)

function setValue(next: string | number) {
  currentValue.value = next
  modelValue.value = next
  emit("update:value", next)
  emit("valueChange", next)
  emit("onValueChange", next)
}

provide(TABS_INJECTION_KEY, {
  currentValue,
  setValue,
  orientation: computed(() => props.orientation),
})
</script>

<template>
  <div
    data-slot="tabs"
    :data-orientation="orientation"
    :class="
      cn(
        'group/tabs flex gap-2 data-[orientation=horizontal]:flex-col',
        props.class,
        props.className
      )
    "
  >
    <slot />
  </div>
</template>

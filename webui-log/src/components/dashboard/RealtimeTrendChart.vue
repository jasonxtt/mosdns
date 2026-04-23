<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as echarts from 'echarts/core'
import type { ECharts, EChartsOption } from 'echarts/core'
import { LineChart } from 'echarts/charts'
import { GridComponent, LegendComponent, TooltipComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import { formatCount, formatLatencyMs } from '../../utils/dashboardFormat'

echarts.use([LineChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer])

const REQUEST_SERIES_NAME = '请求数'
const LATENCY_SERIES_NAME = '平均处理时间'

const props = withDefaults(defineProps<{
  timestamps: string[]
  requestCounts: number[]
  avgLatencyMs: number[]
  showRequestSeries?: boolean
  showLatencySeries?: boolean
}>(), {
  showRequestSeries: true,
  showLatencySeries: true
})

const chartEl = ref<HTMLDivElement | null>(null)

let chart: ECharts | null = null
let resizeObserver: ResizeObserver | null = null

function buildLegendSelected() {
  return {
    [REQUEST_SERIES_NAME]: Boolean(props.showRequestSeries),
    [LATENCY_SERIES_NAME]: Boolean(props.showLatencySeries)
  }
}

function buildGridOption() {
  return {
    top: 14,
    left: props.showRequestSeries ? 48 : 16,
    right: props.showLatencySeries ? 58 : 16,
    bottom: 24
  }
}

function buildYAxisOption() {
  return [
    {
      show: Boolean(props.showRequestSeries),
      type: 'value',
      name: '请求数',
      nameTextStyle: {
        color: '#7f95b9',
        fontSize: 11,
        align: 'left'
      },
      axisLabel: {
        color: '#7f95b9',
        fontSize: 11,
        formatter: (value: number) => formatCount(value)
      },
      axisLine: {
        show: false
      },
      splitLine: {
        lineStyle: {
          color: 'rgba(119, 147, 188, 0.12)'
        }
      }
    },
    {
      show: Boolean(props.showLatencySeries),
      type: 'value',
      name: '处理时间 (ms)',
      nameTextStyle: {
        color: '#7f95b9',
        fontSize: 11,
        align: 'right'
      },
      axisLabel: {
        color: '#7f95b9',
        fontSize: 11,
        formatter: (value: number) => formatLatencyMs(value, false)
      },
      axisLine: {
        show: false
      },
      splitLine: {
        show: false
      }
    }
  ]
}

function buildTooltipHTML(params: any): string {
  const items = Array.isArray(params) ? params : [params]
  if (items.length === 0) {
    return ''
  }

  const timeText = String(items[0]?.axisValueLabel || items[0]?.axisValue || '-')
  const lines = [`<div style="margin-bottom: 4px; color: #b9cfee;">时间：${timeText}</div>`]

  items.forEach((item: any) => {
    const name = String(item?.seriesName || '')
    const marker = String(item?.marker || '')
    if (name === LATENCY_SERIES_NAME) {
      lines.push(`<div>${marker}${name}：${formatLatencyMs(item?.value)}</div>`)
      return
    }
    lines.push(`<div>${marker}${name}：${formatCount(item?.value)}</div>`)
  })

  return lines.join('')
}

function createBaseOption(): EChartsOption {
  return {
    animation: true,
    animationDuration: 520,
    animationEasing: 'cubicOut',
    animationDurationUpdate: 700,
    animationEasingUpdate: 'cubicOut',
    textStyle: {
      color: '#c8d6eb',
      fontFamily: 'IBM Plex Sans, Source Han Sans SC, sans-serif'
    },
    legend: {
      show: false,
      selected: buildLegendSelected(),
      data: [REQUEST_SERIES_NAME, LATENCY_SERIES_NAME]
    },
    tooltip: {
      trigger: 'axis',
      borderWidth: 1,
      borderColor: 'rgba(155, 186, 230, 0.24)',
      backgroundColor: 'rgba(9, 16, 30, 0.94)',
      textStyle: {
        color: '#e8f0ff',
        fontSize: 12
      },
      formatter: buildTooltipHTML,
      axisPointer: {
        type: 'line',
        lineStyle: {
          type: 'dashed',
          color: 'rgba(143, 174, 219, 0.55)',
          width: 1
        }
      }
    },
    grid: buildGridOption(),
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: [],
      axisTick: {
        show: false
      },
      axisLine: {
        lineStyle: {
          color: 'rgba(130, 157, 197, 0.22)'
        }
      },
      axisLabel: {
        color: '#7f95b9',
        fontSize: 11
      },
      splitLine: {
        show: false
      }
    },
    yAxis: buildYAxisOption(),
    series: [
      {
        id: 'request-series',
        name: REQUEST_SERIES_NAME,
        type: 'line',
        yAxisIndex: 0,
        smooth: 0.26,
        showSymbol: false,
        symbol: 'circle',
        symbolSize: 5,
        lineStyle: {
          width: 2,
          color: '#5da8ff'
        },
        itemStyle: {
          color: '#5da8ff'
        },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(93, 168, 255, 0.28)' },
            { offset: 1, color: 'rgba(93, 168, 255, 0.02)' }
          ])
        },
        emphasis: {
          focus: 'series',
          scale: false
        },
        data: []
      },
      {
        id: 'latency-series',
        name: LATENCY_SERIES_NAME,
        type: 'line',
        yAxisIndex: 1,
        smooth: 0.26,
        showSymbol: false,
        symbol: 'circle',
        symbolSize: 5,
        lineStyle: {
          width: 2,
          color: '#40d889'
        },
        itemStyle: {
          color: '#40d889'
        },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(64, 216, 137, 0.22)' },
            { offset: 1, color: 'rgba(64, 216, 137, 0.02)' }
          ])
        },
        emphasis: {
          focus: 'series',
          scale: false
        },
        data: []
      }
    ]
  }
}

function syncSeriesData() {
  if (!chart) {
    return
  }

  chart.setOption({
    legend: {
      selected: buildLegendSelected()
    },
    grid: buildGridOption(),
    yAxis: buildYAxisOption(),
    xAxis: {
      data: props.timestamps
    },
    series: [
      {
        id: 'request-series',
        name: REQUEST_SERIES_NAME,
        data: props.requestCounts
      },
      {
        id: 'latency-series',
        name: LATENCY_SERIES_NAME,
        data: props.avgLatencyMs
      }
    ]
  }, {
    notMerge: false,
    lazyUpdate: true,
    silent: true
  })
}

function resizeChart() {
  chart?.resize()
}

onMounted(() => {
  if (!chartEl.value) {
    return
  }

  chart = echarts.init(chartEl.value, undefined, {
    renderer: 'canvas'
  })
  chart.setOption(createBaseOption())
  syncSeriesData()

  resizeObserver = new ResizeObserver(() => {
    resizeChart()
  })
  resizeObserver.observe(chartEl.value)
  window.addEventListener('resize', resizeChart)
})

watch(
  () => [
    props.timestamps,
    props.requestCounts,
    props.avgLatencyMs,
    props.showRequestSeries,
    props.showLatencySeries
  ],
  () => {
    syncSeriesData()
  },
  { deep: true }
)

onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeChart)
  resizeObserver?.disconnect()
  resizeObserver = null
  chart?.dispose()
  chart = null
})
</script>

<template>
  <div ref="chartEl" class="realtime-trend-chart"></div>
</template>

<style scoped>
.realtime-trend-chart {
  width: 100%;
  height: 220px;
}

@media (max-width: 900px) {
  .realtime-trend-chart {
    height: 190px;
  }
}
</style>

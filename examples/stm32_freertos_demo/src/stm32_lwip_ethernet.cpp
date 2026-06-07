#include <Arduino.h>

#include "stm32_lwip_ethernet.hpp"

extern "C" {
#include <lwip/etharp.h>
#include <lwip/init.h>
#include <lwip/netif.h>
#include <lwip/pbuf.h>
#include <lwip/tcpip.h>
#include <netif/ethernet.h>
}

#include <FreeRTOS.h>
#include <semphr.h>
#include <task.h>
#include <stddef.h>
#include <string.h>

#ifndef STM32_ETH_IP_ADDR0
#define STM32_ETH_IP_ADDR0 192
#endif
#ifndef STM32_ETH_IP_ADDR1
#define STM32_ETH_IP_ADDR1 168
#endif
#ifndef STM32_ETH_IP_ADDR2
#define STM32_ETH_IP_ADDR2 0
#endif
#ifndef STM32_ETH_IP_ADDR3
#define STM32_ETH_IP_ADDR3 42
#endif

#ifndef STM32_ETH_GW_ADDR0
#define STM32_ETH_GW_ADDR0 192
#endif
#ifndef STM32_ETH_GW_ADDR1
#define STM32_ETH_GW_ADDR1 168
#endif
#ifndef STM32_ETH_GW_ADDR2
#define STM32_ETH_GW_ADDR2 0
#endif
#ifndef STM32_ETH_GW_ADDR3
#define STM32_ETH_GW_ADDR3 1
#endif

#ifndef STM32_ETH_NETMASK_ADDR0
#define STM32_ETH_NETMASK_ADDR0 255
#endif
#ifndef STM32_ETH_NETMASK_ADDR1
#define STM32_ETH_NETMASK_ADDR1 255
#endif
#ifndef STM32_ETH_NETMASK_ADDR2
#define STM32_ETH_NETMASK_ADDR2 255
#endif
#ifndef STM32_ETH_NETMASK_ADDR3
#define STM32_ETH_NETMASK_ADDR3 0
#endif

static constexpr uint32_t PHY_ADDR = 0;
static constexpr uint32_t PHY_BCR = 0;
static constexpr uint32_t PHY_BSR = 1;
static constexpr uint32_t PHY_SR = 31;
static constexpr uint32_t PHY_BCR_RESET = 0x8000;
static constexpr uint32_t PHY_BCR_AUTONEGO_EN = 0x1000;
static constexpr uint32_t PHY_BCR_RESTART_AUTONEGO = 0x0200;
static constexpr uint32_t PHY_BSR_LINK_STATUS = 0x0004;
static constexpr uint32_t PHY_BSR_AUTONEGO_COMPLETE = 0x0020;
static constexpr uint32_t PHY_SR_SPEED_DUPLEX_MASK = 0x001C;
static constexpr uint32_t PHY_SR_100BTX_FULLDUPLEX = 0x0018;
static constexpr uint32_t PHY_SR_100BTX_HALFDUPLEX = 0x0008;

static constexpr size_t RX_BUFFER_SIZE = 1536;
static constexpr size_t RX_BUFFER_COUNT = ETH_RX_DESC_CNT + 4;
static constexpr size_t TX_BUFFER_SIZE = 1536;
static constexpr uint32_t ETH_TIMEOUT_MS = 100;

struct RxBuffer {
    RxBuffer *next;
    uint16_t len;
    alignas(32) uint8_t data[RX_BUFFER_SIZE];
};

static ETH_HandleTypeDef g_eth;
static ETH_TxPacketConfigTypeDef g_txConfig;
static ETH_DMADescTypeDef g_txDmaDesc[ETH_TX_DESC_CNT] __attribute__((aligned(32)));
static ETH_DMADescTypeDef g_rxDmaDesc[ETH_RX_DESC_CNT] __attribute__((aligned(32)));
static uint8_t g_macAddress[6] = {0x02, 0x00, 0x00, 0x12, 0x34, 0x56};
static struct netif g_netif;
static SemaphoreHandle_t g_tcpipInitDone = nullptr;
static SemaphoreHandle_t g_txMutex = nullptr;
static RxBuffer g_rxBuffers[RX_BUFFER_COUNT];
static RxBuffer *g_freeRxBuffers = nullptr;
static uint8_t g_txBuffer[TX_BUFFER_SIZE] __attribute__((aligned(32)));
static char g_ipString[16] = "0.0.0.0";

static void cleanDCache(const void *addr, size_t len) {
#if (__DCACHE_PRESENT == 1U)
    const uintptr_t start = reinterpret_cast<uintptr_t>(addr) & ~static_cast<uintptr_t>(31);
    const uintptr_t end = (reinterpret_cast<uintptr_t>(addr) + len + 31) & ~static_cast<uintptr_t>(31);
    SCB_CleanDCache_by_Addr(reinterpret_cast<uint32_t *>(start), static_cast<int32_t>(end - start));
#else
    (void)addr;
    (void)len;
#endif
}

static void invalidateDCache(const void *addr, size_t len) {
#if (__DCACHE_PRESENT == 1U)
    const uintptr_t start = reinterpret_cast<uintptr_t>(addr) & ~static_cast<uintptr_t>(31);
    const uintptr_t end = (reinterpret_cast<uintptr_t>(addr) + len + 31) & ~static_cast<uintptr_t>(31);
    SCB_InvalidateDCache_by_Addr(reinterpret_cast<uint32_t *>(start), static_cast<int32_t>(end - start));
#else
    (void)addr;
    (void)len;
#endif
}

static RxBuffer *rxBufferFromData(uint8_t *data) {
    return reinterpret_cast<RxBuffer *>(reinterpret_cast<uint8_t *>(data) - offsetof(RxBuffer, data));
}

static RxBuffer *allocRxBuffer() {
    taskENTER_CRITICAL();
    RxBuffer *buffer = g_freeRxBuffers;
    if (buffer != nullptr) {
        g_freeRxBuffers = buffer->next;
        buffer->next = nullptr;
        buffer->len = 0;
    }
    taskEXIT_CRITICAL();
    return buffer;
}

static void freeRxBuffer(RxBuffer *buffer) {
    if (buffer == nullptr) {
        return;
    }
    taskENTER_CRITICAL();
    buffer->next = g_freeRxBuffers;
    g_freeRxBuffers = buffer;
    taskEXIT_CRITICAL();
}

static void freeRxChain(RxBuffer *buffer) {
    while (buffer != nullptr) {
        RxBuffer *next = buffer->next;
        freeRxBuffer(buffer);
        buffer = next;
    }
}

extern "C" void HAL_ETH_RxAllocateCallback(uint8_t **buff) {
    RxBuffer *buffer = allocRxBuffer();
    *buff = (buffer != nullptr) ? buffer->data : nullptr;
}

extern "C" void HAL_ETH_RxLinkCallback(void **pStart, void **pEnd, uint8_t *buff, uint16_t length) {
    RxBuffer *buffer = rxBufferFromData(buff);
    buffer->len = length;
    buffer->next = nullptr;

    if (*pStart == nullptr) {
        *pStart = buffer;
    } else {
        static_cast<RxBuffer *>(*pEnd)->next = buffer;
    }
    *pEnd = buffer;
}

extern "C" void HAL_ETH_MspInit(ETH_HandleTypeDef *heth) {
    if (heth->Instance != ETH) {
        return;
    }

    __HAL_RCC_ETH1MAC_CLK_ENABLE();
    __HAL_RCC_ETH1TX_CLK_ENABLE();
    __HAL_RCC_ETH1RX_CLK_ENABLE();
    __HAL_RCC_GPIOA_CLK_ENABLE();
    __HAL_RCC_GPIOB_CLK_ENABLE();
    __HAL_RCC_GPIOC_CLK_ENABLE();
    __HAL_RCC_GPIOG_CLK_ENABLE();

    GPIO_InitTypeDef gpio = {};
    gpio.Mode = GPIO_MODE_AF_PP;
    gpio.Pull = GPIO_NOPULL;
    gpio.Speed = GPIO_SPEED_FREQ_VERY_HIGH;
    gpio.Alternate = GPIO_AF11_ETH;

    gpio.Pin = GPIO_PIN_1 | GPIO_PIN_2 | GPIO_PIN_7;
    HAL_GPIO_Init(GPIOA, &gpio);

    gpio.Pin = GPIO_PIN_13;
    HAL_GPIO_Init(GPIOB, &gpio);

    gpio.Pin = GPIO_PIN_1 | GPIO_PIN_4 | GPIO_PIN_5;
    HAL_GPIO_Init(GPIOC, &gpio);

    gpio.Pin = GPIO_PIN_11 | GPIO_PIN_13;
    HAL_GPIO_Init(GPIOG, &gpio);
}

static bool readPhy(uint32_t reg, uint32_t *value) {
    return HAL_ETH_ReadPHYRegister(&g_eth, PHY_ADDR, reg, value) == HAL_OK;
}

static bool writePhy(uint32_t reg, uint32_t value) {
    return HAL_ETH_WritePHYRegister(&g_eth, PHY_ADDR, reg, value) == HAL_OK;
}

static bool waitForPhyLink() {
    uint32_t value = 0;

    if (!writePhy(PHY_BCR, PHY_BCR_RESET)) {
        Serial.println("PHY reset command failed");
        return false;
    }

    const uint32_t resetStart = millis();
    do {
        if (!readPhy(PHY_BCR, &value)) {
            return false;
        }
        vTaskDelay(pdMS_TO_TICKS(10));
    } while ((value & PHY_BCR_RESET) != 0 && millis() - resetStart < 1000);

    writePhy(PHY_BCR, PHY_BCR_AUTONEGO_EN | PHY_BCR_RESTART_AUTONEGO);

    const uint32_t linkStart = millis();
    do {
        if (!readPhy(PHY_BSR, &value)) {
            return false;
        }
        if ((value & PHY_BSR_LINK_STATUS) != 0 && (value & PHY_BSR_AUTONEGO_COMPLETE) != 0) {
            return true;
        }
        vTaskDelay(pdMS_TO_TICKS(100));
    } while (millis() - linkStart < 10000);

    Serial.println("PHY link/autonegotiation timeout");
    return false;
}

static void configureMacFromPhy() {
    ETH_MACConfigTypeDef macConfig = {};
    uint32_t phyStatus = 0;

    HAL_ETH_GetMACConfig(&g_eth, &macConfig);
    if (readPhy(PHY_SR, &phyStatus)) {
        const uint32_t speedDuplex = phyStatus & PHY_SR_SPEED_DUPLEX_MASK;
        macConfig.Speed = (speedDuplex == PHY_SR_100BTX_FULLDUPLEX || speedDuplex == PHY_SR_100BTX_HALFDUPLEX) ? ETH_SPEED_100M : ETH_SPEED_10M;
        macConfig.DuplexMode = (speedDuplex == PHY_SR_100BTX_FULLDUPLEX) ? ETH_FULLDUPLEX_MODE : ETH_HALFDUPLEX_MODE;
    } else {
        macConfig.Speed = ETH_SPEED_100M;
        macConfig.DuplexMode = ETH_FULLDUPLEX_MODE;
    }
    HAL_ETH_SetMACConfig(&g_eth, &macConfig);
}

static err_t lowLevelOutput(struct netif *netif, struct pbuf *p) {
    (void)netif;
    if (p->tot_len > TX_BUFFER_SIZE) {
        return ERR_BUF;
    }
    if (xSemaphoreTake(g_txMutex, pdMS_TO_TICKS(ETH_TIMEOUT_MS)) != pdTRUE) {
        return ERR_TIMEOUT;
    }

    uint8_t *dst = g_txBuffer;
    for (struct pbuf *q = p; q != nullptr; q = q->next) {
        memcpy(dst, q->payload, q->len);
        dst += q->len;
    }

    cleanDCache(g_txBuffer, p->tot_len);

    ETH_BufferTypeDef txBuffer = {};
    txBuffer.buffer = g_txBuffer;
    txBuffer.len = p->tot_len;
    txBuffer.next = nullptr;

    g_txConfig.Length = p->tot_len;
    g_txConfig.TxBuffer = &txBuffer;
    g_txConfig.pData = nullptr;

    const HAL_StatusTypeDef status = HAL_ETH_Transmit(&g_eth, &g_txConfig, ETH_TIMEOUT_MS);
    xSemaphoreGive(g_txMutex);
    return status == HAL_OK ? ERR_OK : ERR_IF;
}

static err_t ethernetifInit(struct netif *netif) {
    netif->name[0] = 's';
    netif->name[1] = 't';
    netif->output = etharp_output;
    netif->linkoutput = lowLevelOutput;
    netif->mtu = 1500;
    netif->flags = NETIF_FLAG_BROADCAST | NETIF_FLAG_ETHARP | NETIF_FLAG_LINK_UP;
    netif->hwaddr_len = ETH_HWADDR_LEN;
    memcpy(netif->hwaddr, g_macAddress, sizeof(g_macAddress));
    return ERR_OK;
}

static void rxTask(void *parameter) {
    (void)parameter;

    for (;;) {
        void *appBuff = nullptr;
        if (HAL_ETH_ReadData(&g_eth, &appBuff) == HAL_OK && appBuff != nullptr) {
            RxBuffer *chain = static_cast<RxBuffer *>(appBuff);
            uint16_t totalLen = 0;
            for (RxBuffer *buffer = chain; buffer != nullptr; buffer = buffer->next) {
                invalidateDCache(buffer->data, buffer->len);
                totalLen += buffer->len;
            }

            struct pbuf *p = pbuf_alloc(PBUF_RAW, totalLen, PBUF_POOL);
            if (p != nullptr) {
                uint16_t offset = 0;
                for (RxBuffer *buffer = chain; buffer != nullptr; buffer = buffer->next) {
                    pbuf_take_at(p, buffer->data, buffer->len, offset);
                    offset += buffer->len;
                }
                if (g_netif.input(p, &g_netif) != ERR_OK) {
                    pbuf_free(p);
                }
            }
            freeRxChain(chain);
        } else {
            vTaskDelay(pdMS_TO_TICKS(1));
        }
    }
}

static void tcpipInitDone(void *arg) {
    xSemaphoreGive(static_cast<SemaphoreHandle_t>(arg));
}

bool stm32StartEthernet() {
    static bool started = false;
    if (started) {
        return true;
    }

    for (size_t i = 0; i < RX_BUFFER_COUNT; i++) {
        freeRxBuffer(&g_rxBuffers[i]);
    }

    g_txMutex = xSemaphoreCreateMutex();
    g_tcpipInitDone = xSemaphoreCreateBinary();
    if (g_txMutex == nullptr || g_tcpipInitDone == nullptr) {
        Serial.println("Failed to create Ethernet RTOS objects");
        return false;
    }

    g_eth.Instance = ETH;
    g_eth.Init.MACAddr = g_macAddress;
    g_eth.Init.MediaInterface = HAL_ETH_RMII_MODE;
    g_eth.Init.TxDesc = g_txDmaDesc;
    g_eth.Init.RxDesc = g_rxDmaDesc;
    g_eth.Init.RxBuffLen = RX_BUFFER_SIZE;

    g_txConfig.Attributes = ETH_TX_PACKETS_FEATURES_CSUM | ETH_TX_PACKETS_FEATURES_CRCPAD;
    g_txConfig.ChecksumCtrl = ETH_CHECKSUM_IPHDR_PAYLOAD_INSERT_PHDR_CALC;
    g_txConfig.CRCPadCtrl = ETH_CRC_PAD_INSERT;

    if (HAL_ETH_Init(&g_eth) != HAL_OK) {
        Serial.printf("HAL_ETH_Init failed, error=0x%08lx\n", g_eth.ErrorCode);
        return false;
    }

    if (!waitForPhyLink()) {
        return false;
    }
    configureMacFromPhy();

    tcpip_init(tcpipInitDone, g_tcpipInitDone);
    if (xSemaphoreTake(g_tcpipInitDone, pdMS_TO_TICKS(3000)) != pdTRUE) {
        Serial.println("lwIP tcpip_init timeout");
        return false;
    }

    ip4_addr_t ipaddr;
    ip4_addr_t netmask;
    ip4_addr_t gw;
    IP4_ADDR(&ipaddr, STM32_ETH_IP_ADDR0, STM32_ETH_IP_ADDR1, STM32_ETH_IP_ADDR2, STM32_ETH_IP_ADDR3);
    IP4_ADDR(&netmask, STM32_ETH_NETMASK_ADDR0, STM32_ETH_NETMASK_ADDR1, STM32_ETH_NETMASK_ADDR2, STM32_ETH_NETMASK_ADDR3);
    IP4_ADDR(&gw, STM32_ETH_GW_ADDR0, STM32_ETH_GW_ADDR1, STM32_ETH_GW_ADDR2, STM32_ETH_GW_ADDR3);

    if (netif_add(&g_netif, &ipaddr, &netmask, &gw, nullptr, ethernetifInit, tcpip_input) == nullptr) {
        Serial.println("netif_add failed");
        return false;
    }
    netif_set_default(&g_netif);
    netif_set_up(&g_netif);
    netif_set_link_up(&g_netif);

    if (HAL_ETH_Start(&g_eth) != HAL_OK) {
        Serial.println("HAL_ETH_Start failed");
        return false;
    }

    if (xTaskCreate(rxTask, "eth_rx", 2048, nullptr, configMAX_PRIORITIES - 2, nullptr) != pdPASS) {
        Serial.println("Failed to create Ethernet RX task");
        return false;
    }

    snprintf(g_ipString, sizeof(g_ipString), "%u.%u.%u.%u", STM32_ETH_IP_ADDR0, STM32_ETH_IP_ADDR1, STM32_ETH_IP_ADDR2, STM32_ETH_IP_ADDR3);
    Serial.printf("Ethernet up: IP %s\n", g_ipString);
    started = true;
    return true;
}

const char *stm32EthernetIpString() { return g_ipString; }

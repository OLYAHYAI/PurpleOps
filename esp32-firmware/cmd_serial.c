/**
 * @file cmd_serial.c
 * @brief Interface de commande UART pour piloter le firmware risinek par le
 *        port série (USB CDC/CH9102), sans passer par l'UI web WiFi.
 *
 * Ajout non-intrusif : réutilise toute la machinerie d'attaque de risinek
 * (events WEBSERVER_EVENT_ATTACK_REQUEST / RESET + ap_scanner). L'UI web reste
 * active en parallèle. Les réponses machine sont préfixées "#WIDS " pour être
 * filtrées des logs ESP-IDF côté hôte (le Pi via wids-ctl).
 *
 * Protocole (une commande par ligne, terminée CR) :
 *   ping                              -> #WIDS pong
 *   scan                             -> #WIDS AP <i> <bssid> ch=<n> rssi=<n> ssid=<s> ... #WIDS SCAN done
 *   deauth <bssid|idx> [timeout] [method]   method 0=rogueap 1=broadcast(defaut) 2=combine
 *   capture <bssid|idx> [timeout]    -> handshake WPA/WPA2
 *   pmkid <bssid|idx> [timeout]
 *   attack <type> <idx> <method> <timeout>   (brut)
 *   status                           -> #WIDS STATUS state=.. type=.. size=..
 *   stop                             -> #WIDS OK stop
 *
 * Périmètre : réseau de l'utilisateur uniquement (contrôle par le Pi/wids-ctl).
 */
#include <stdio.h>
#include <string.h>
#include <ctype.h>
#include <stdlib.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_event.h"
#include "driver/uart.h"
#include "esp_vfs_dev.h"

#include "attack.h"
#include "wifi_controller.h"   /* inclut transitivement ap_scanner.h (wifictl_ap_records_t, scan) */
#include "webserver.h"

#define PFX "#WIDS "

static int parse_mac(const char *s, uint8_t out[6]) {
    unsigned v[6];
    if (sscanf(s, "%x:%x:%x:%x:%x:%x", &v[0], &v[1], &v[2], &v[3], &v[4], &v[5]) != 6) return -1;
    for (int i = 0; i < 6; i++) out[i] = (uint8_t)v[i];
    return 0;
}

static int find_ap_by_bssid(const uint8_t bssid[6]) {
    const wifictl_ap_records_t *recs = wifictl_get_ap_records();
    if (!recs) return -1;
    for (unsigned i = 0; i < recs->count; i++) {
        if (memcmp(recs->records[i].bssid, bssid, 6) == 0) return (int)i;
    }
    return -1;
}

static void post_attack(uint8_t idx, uint8_t type, uint8_t method, uint8_t timeout) {
    attack_request_t req = { .ap_record_id = idx, .type = type, .method = method, .timeout = timeout };
    esp_event_post(WEBSERVER_EVENTS, WEBSERVER_EVENT_ATTACK_REQUEST, &req, sizeof(req), portMAX_DELAY);
}

static void do_scan_and_list(void) {
    wifictl_scan_nearby_aps();
    const wifictl_ap_records_t *recs = wifictl_get_ap_records();
    unsigned n = recs ? recs->count : 0;
    printf(PFX "SCAN count=%u\n", n);
    for (unsigned i = 0; i < n; i++) {
        const wifi_ap_record_t *r = &recs->records[i];
        printf(PFX "AP %u %02x:%02x:%02x:%02x:%02x:%02x ch=%u rssi=%d ssid=%s\n",
               i, r->bssid[0], r->bssid[1], r->bssid[2], r->bssid[3], r->bssid[4], r->bssid[5],
               r->primary, r->rssi, (const char *)r->ssid);
    }
    printf(PFX "SCAN done\n");
}

/* resout un argument bssid-ou-index en index de scan ; -1 si introuvable */
static int resolve_target(const char *arg) {
    uint8_t mac[6];
    if (parse_mac(arg, mac) == 0) {
        do_scan_and_list();           /* rafraichit la liste pour resoudre le bssid */
        return find_ap_by_bssid(mac);
    }
    return atoi(arg);
}

static void handle_line(char *line) {
    char *p = line;
    while (*p && isspace((unsigned char)*p)) p++;
    size_t n = strlen(p);
    while (n > 0 && isspace((unsigned char)p[n - 1])) p[--n] = 0;
    if (*p == 0) return;

    char cmd[16] = {0}, a1[64] = {0}, a2[16] = {0}, a3[16] = {0}, a4[16] = {0};
    int nf = sscanf(p, "%15s %63s %15s %15s %15s", cmd, a1, a2, a3, a4);
    if (nf < 1) return;

    if (strcasecmp(cmd, "ping") == 0) { printf(PFX "pong\n"); return; }
    if (strcasecmp(cmd, "scan") == 0) { do_scan_and_list(); return; }
    if (strcasecmp(cmd, "status") == 0) {
        const attack_status_t *st = attack_get_status();
        printf(PFX "STATUS state=%u type=%u size=%u\n", st->state, st->type, st->content_size);
        return;
    }
    if (strcasecmp(cmd, "stop") == 0) {
        esp_event_post(WEBSERVER_EVENTS, WEBSERVER_EVENT_ATTACK_RESET, NULL, 0, portMAX_DELAY);
        printf(PFX "OK stop\n");
        return;
    }
    if (strcasecmp(cmd, "deauth") == 0 && nf >= 2) {
        uint8_t timeout = (nf >= 3) ? (uint8_t)atoi(a2) : 30;
        uint8_t method  = (nf >= 4) ? (uint8_t)atoi(a3) : 1; /* BROADCAST */
        int idx = resolve_target(a1);
        if (idx < 0) { printf(PFX "ERR bssid_not_found\n"); return; }
        post_attack((uint8_t)idx, ATTACK_TYPE_DOS, method, timeout);
        printf(PFX "OK deauth idx=%d method=%u timeout=%u\n", idx, method, timeout);
        return;
    }
    if (strcasecmp(cmd, "capture") == 0 && nf >= 2) {
        uint8_t timeout = (nf >= 3) ? (uint8_t)atoi(a2) : 60;
        int idx = resolve_target(a1);
        if (idx < 0) { printf(PFX "ERR bssid_not_found\n"); return; }
        post_attack((uint8_t)idx, ATTACK_TYPE_HANDSHAKE, 0, timeout);
        printf(PFX "OK capture idx=%d timeout=%u\n", idx, timeout);
        return;
    }
    if (strcasecmp(cmd, "pmkid") == 0 && nf >= 2) {
        uint8_t timeout = (nf >= 3) ? (uint8_t)atoi(a2) : 30;
        int idx = resolve_target(a1);
        if (idx < 0) { printf(PFX "ERR bssid_not_found\n"); return; }
        post_attack((uint8_t)idx, ATTACK_TYPE_PMKID, 0, timeout);
        printf(PFX "OK pmkid idx=%d timeout=%u\n", idx, timeout);
        return;
    }
    if (strcasecmp(cmd, "attack") == 0 && nf >= 5) {
        post_attack((uint8_t)atoi(a2), (uint8_t)atoi(a1), (uint8_t)atoi(a3), (uint8_t)atoi(a4));
        printf(PFX "OK attack\n");
        return;
    }
    printf(PFX "ERR unknown_cmd\n");
}

static void cmd_serial_task(void *arg) {
    /* entree ligne bloquante sur UART0 (le port USB-serie) */
    setvbuf(stdin, NULL, _IONBF, 0);
    esp_vfs_dev_uart_set_rx_line_endings(ESP_LINE_ENDINGS_CR);
    esp_vfs_dev_uart_set_tx_line_endings(ESP_LINE_ENDINGS_CRLF);
    uart_driver_install(UART_NUM_0, 256, 0, 0, NULL, 0);
    esp_vfs_dev_uart_use_driver(UART_NUM_0);

    printf(PFX "ready\n");
    char lbuf[160];
    while (1) {
        if (fgets(lbuf, sizeof(lbuf), stdin) != NULL) {
            handle_line(lbuf);
        } else {
            vTaskDelay(pdMS_TO_TICKS(20));
        }
    }
}

void cmd_serial_start(void) {
    xTaskCreate(cmd_serial_task, "cmd_serial", 4096, NULL, 5, NULL);
}

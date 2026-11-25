/* SPDX-License-Identifier: MIT OR GPL-3.0-only */
/* tls_rustls_glue.c
** Accessor functions for xmpp_conn_t fields that Rust can't access directly
** due to opaque struct definition
*/

#include "libstrophe/common.h"
#include "libstrophe/sock.h"
#include "libstrophe/tls.h"

/* Get socket fd from connection */
int xmpp_conn_get_sock(xmpp_conn_t *conn)
{
    if (!conn)
        return -1;
    return conn->sock;
}

/* Get domain from connection */
const char *xmpp_conn_get_domain(xmpp_conn_t *conn)
{
    if (!conn)
        return NULL;
    return conn->domain;
}

/* Get tls field from connection */
tls_t *xmpp_conn_get_tls(xmpp_conn_t *conn)
{
    if (!conn)
        return NULL;
    return conn->tls;
}

/* Set tls field in connection */
void xmpp_conn_set_tls(xmpp_conn_t *conn, tls_t *tls)
{
    if (conn)
        conn->tls = tls;
}

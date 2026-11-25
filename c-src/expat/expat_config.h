/* Minimal expat_config.h for vendored build */
#ifndef EXPAT_CONFIG_H
#define EXPAT_CONFIG_H

/* Static build */
#define XML_STATIC 1

/* Use poor entropy (avoid /dev/urandom detection issues on exotic targets) */
#define XML_POOR_ENTROPY 1

/* Enable features */
#define XML_DTD 1
#define XML_GE 1
#define XML_NS 1
#define XML_CONTEXT_BYTES 1024

/* Standard C functions */
#define HAVE_MEMMOVE 1

/* Large size support - let expat_external.h define the types */
#define XML_LARGE_SIZE 1

#endif /* EXPAT_CONFIG_H */

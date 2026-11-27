OPENRESTY_PREFIX=/usr/local/openresty

PREFIX :=          /usr/local
LUA_LIB_DIR :=     $(OPENRESTY_PREFIX)/lualib
INSTALL := install

.PHONY: all install

all:

install: all
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/auxiliary
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include/evp
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include/x509
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/x509
	$(INSTALL) -d $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/x509/extension
	$(INSTALL) lib/resty/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty
	$(INSTALL) lib/resty/openssl/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl
	$(INSTALL) lib/resty/openssl/auxiliary/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/auxiliary
	$(INSTALL) lib/resty/openssl/include/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include
	$(INSTALL) lib/resty/openssl/include/evp/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include/evp
	$(INSTALL) lib/resty/openssl/include/x509/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/include/x509
	$(INSTALL) lib/resty/openssl/x509/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/x509
	$(INSTALL) lib/resty/openssl/x509/extension/*.lua $(DESTDIR)$(LUA_LIB_DIR)/resty/openssl/x509/extension

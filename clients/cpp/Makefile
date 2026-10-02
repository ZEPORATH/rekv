CXX ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Werror
CPPFLAGS += -Iinclude

all: read_value watch_baud_rate

read_value: src/config_client.cpp examples/read_value.cpp include/rekv/config_client.hpp
	$(CXX) $(CPPFLAGS) $(CXXFLAGS) src/config_client.cpp examples/read_value.cpp -o $@

watch_baud_rate: src/config_client.cpp examples/watch_baud_rate.cpp include/rekv/config_client.hpp
	$(CXX) $(CPPFLAGS) $(CXXFLAGS) src/config_client.cpp examples/watch_baud_rate.cpp -o $@

clean:
	rm -f read_value watch_baud_rate

.PHONY: all clean
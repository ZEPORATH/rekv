package config

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strconv"
)

type ValueType string

const (
	TypeString      ValueType = "string"
	TypeInteger     ValueType = "integer"
	TypeFloat       ValueType = "float"
	TypeBoolean     ValueType = "boolean"
	TypeObject      ValueType = "object"
	TypeArray       ValueType = "array"
	TypeStringList  ValueType = "str_list"
	TypeNumericList ValueType = "numeric_list"
	TypeNull        ValueType = "null"
)

type Value struct {
	Type  ValueType `json:"type"`
	Value any       `json:"value"`
}

func (value *Value) UnmarshalJSON(data []byte) error {
	var wire struct {
		Type  ValueType       `json:"type"`
		Value json.RawMessage `json:"value"`
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		return err
	}
	value.Type = wire.Type
	switch wire.Type {
	case TypeInteger:
		var number json.Number
		decoder := json.NewDecoder(bytes.NewReader(wire.Value))
		decoder.UseNumber()
		if err := decoder.Decode(&number); err != nil {
			return err
		}
		if integer, err := number.Int64(); err == nil {
			value.Value = integer
		} else if unsigned, unsignedErr := strconv.ParseUint(number.String(), 10, 64); unsignedErr == nil {
			value.Value = unsigned
		} else {
			return fmt.Errorf("integer value is outside the supported range")
		}
	case TypeFloat:
		var number float64
		if err := json.Unmarshal(wire.Value, &number); err != nil {
			return err
		}
		value.Value = number
	case TypeString:
		var stringValue string
		if err := json.Unmarshal(wire.Value, &stringValue); err != nil {
			return err
		}
		value.Value = stringValue
	case TypeBoolean:
		var booleanValue bool
		if err := json.Unmarshal(wire.Value, &booleanValue); err != nil {
			return err
		}
		value.Value = booleanValue
	case TypeStringList:
		var list []string
		if err := json.Unmarshal(wire.Value, &list); err != nil {
			return err
		}
		value.Value = list
	case TypeNumericList:
		var list []json.Number
		decoder := json.NewDecoder(bytes.NewReader(wire.Value))
		decoder.UseNumber()
		if err := decoder.Decode(&list); err != nil {
			return err
		}
		value.Value = list
	case TypeObject:
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(wire.Value, &fields); err != nil {
			return err
		}
		decoded := make(map[string]Value, len(fields))
		for name, field := range fields {
			var child Value
			if err := json.Unmarshal(field, &child); err != nil {
				return fmt.Errorf("object field %q: %w", name, err)
			}
			decoded[name] = child
		}
		value.Value = decoded
	case TypeArray:
		var items []json.RawMessage
		if err := json.Unmarshal(wire.Value, &items); err != nil {
			return err
		}
		decoded := make([]Value, len(items))
		for index, item := range items {
			if err := json.Unmarshal(item, &decoded[index]); err != nil {
				return fmt.Errorf("array item %d: %w", index, err)
			}
		}
		value.Value = decoded
	case TypeNull:
		if value.Value != nil {
			return fmt.Errorf("null value must contain nil")
		}
		if !bytes.Equal(bytes.TrimSpace(wire.Value), []byte("null")) {
			return fmt.Errorf("null value must be null")
		}
		value.Value = nil
	default:
		return fmt.Errorf("unsupported runtime type %q", wire.Type)
	}
	return nil
}

func String(value string) Value { return Value{Type: TypeString, Value: value} }
func Int(value int64) Value     { return Value{Type: TypeInteger, Value: value} }
func Float(value float64) Value { return Value{Type: TypeFloat, Value: value} }
func Bool(value bool) Value     { return Value{Type: TypeBoolean, Value: value} }
func Object(value map[string]Value) Value {
	return Value{Type: TypeObject, Value: value}
}
func Array(value []Value) Value { return Value{Type: TypeArray, Value: value} }
func StrList(value []string) Value {
	return Value{Type: TypeStringList, Value: value}
}
func NumericList(value []json.Number) Value {
	return Value{Type: TypeNumericList, Value: value}
}
func Null() Value { return Value{Type: TypeNull, Value: nil} }

func (value Value) Validate() error {
	switch value.Type {
	case TypeString:
		if _, ok := value.Value.(string); !ok {
			return fmt.Errorf("string value must contain a string")
		}
	case TypeInteger:
		switch value.Value.(type) {
		case int, int8, int16, int32, int64, uint, uint8, uint16, uint32, uint64, json.Number:
		default:
			return fmt.Errorf("integer value must contain an integer")
		}
	case TypeFloat:
		if _, ok := value.Value.(float64); !ok {
			return fmt.Errorf("float value must contain a number")
		}
	case TypeBoolean:
		if _, ok := value.Value.(bool); !ok {
			return fmt.Errorf("boolean value must contain a boolean")
		}
	case TypeStringList:
		if _, ok := value.Value.([]string); !ok {
			return fmt.Errorf("str_list value must contain []string")
		}
	case TypeNumericList:
		switch values := value.Value.(type) {
		case []json.Number:
			for _, number := range values {
				if _, err := number.Float64(); err != nil {
					return fmt.Errorf("numeric_list contains an invalid number: %w", err)
				}
			}
		case []float64, []int, []int64, []uint64:
		default:
			return fmt.Errorf("numeric_list value must contain numeric values")
		}
	case TypeObject:
		if fields, ok := value.Value.(map[string]Value); ok {
			for name, field := range fields {
				if err := field.Validate(); err != nil {
					return fmt.Errorf("object field %q: %w", name, err)
				}
			}
		}
	case TypeArray:
		if values, ok := value.Value.([]Value); ok {
			for index, item := range values {
				if err := item.Validate(); err != nil {
					return fmt.Errorf("array item %d: %w", index, err)
				}
			}
		}
	case TypeNull:
	default:
		return fmt.Errorf("unsupported runtime type %q", value.Type)
	}
	return nil
}

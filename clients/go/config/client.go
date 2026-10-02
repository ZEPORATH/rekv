package config

import (
	"context"
	"encoding/json"
	"fmt"
	"sync/atomic"

	rekvpb "github.com/raut/rekv/clients/go/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type Client struct {
	connection *grpc.ClientConn
	service    rekvpb.RekvServiceClient
	requestID  atomic.Uint64
}

type RPCError struct {
	Code    string
	Message string
}

func (rpcError *RPCError) Error() string {
	return fmt.Sprintf("%s: %s", rpcError.Code, rpcError.Message)
}

func NewClient(target string) (*Client, error) {
	connection, err := grpc.NewClient(target, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, err
	}
	client := &Client{connection: connection, service: rekvpb.NewRekvServiceClient(connection)}
	client.requestID.Store(1)
	return client, nil
}

func (client *Client) Close() error { return client.connection.Close() }

func (client *Client) Get(ctx context.Context, path string) (Value, error) {
	result, err := client.call(ctx, "get", path, nil)
	if err != nil {
		return Value{}, err
	}
	var value Value
	if err := json.Unmarshal([]byte(result), &value); err != nil {
		return Value{}, err
	}
	return value, value.Validate()
}

func (client *Client) Set(ctx context.Context, path string, value Value) error {
	if err := value.Validate(); err != nil {
		return err
	}
	return client.invoke(ctx, "set", path, &value)
}

func (client *Client) Delete(ctx context.Context, path string) error {
	return client.invoke(ctx, "delete", path, nil)
}

func (client *Client) Restore(ctx context.Context, path string) error {
	return client.invoke(ctx, "restore", path, nil)
}

func (client *Client) List(ctx context.Context, path string) ([]string, error) {
	result, err := client.call(ctx, "list", path, nil)
	if err != nil {
		return nil, err
	}
	var values []string
	if err := json.Unmarshal([]byte(result), &values); err != nil {
		return nil, err
	}
	return values, nil
}

func (client *Client) Watch(ctx context.Context, path string) (rekvpb.RekvService_WatchClient, error) {
	return client.service.Watch(ctx, &rekvpb.WatchRequest{Path: path})
}

func (client *Client) invoke(ctx context.Context, method, path string, value *Value) error {
	_, err := client.call(ctx, method, path, value)
	return err
}

func (client *Client) call(ctx context.Context, method, path string, value *Value) (string, error) {
	var encodedValue string
	if value != nil {
		encoded, err := json.Marshal(value)
		if err != nil {
			return "", err
		}
		encodedValue = string(encoded)
	}
	response, err := client.service.Call(ctx, &rekvpb.RpcCallRequest{
		Id:        client.requestID.Add(1),
		Method:    method,
		Path:      path,
		JsonValue: encodedValue,
	})
	if err != nil {
		return "", err
	}
	if !response.Ok {
		return "", &RPCError{Code: response.ErrorCode, Message: response.ErrorMessage}
	}
	return response.ResultJson, nil
}

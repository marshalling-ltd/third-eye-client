# \DeviceHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_device_handler**](DeviceHandlerApi.md#create_device_handler) | **POST** /api/v1/devices | 
[**delete_device_handler**](DeviceHandlerApi.md#delete_device_handler) | **DELETE** /api/v1/devices/{id} | 
[**device_list_handler**](DeviceHandlerApi.md#device_list_handler) | **GET** /api/v1/devices | 
[**device_list_search_handler**](DeviceHandlerApi.md#device_list_search_handler) | **GET** /api/v1/devices/search | 
[**edit_device_handler**](DeviceHandlerApi.md#edit_device_handler) | **PATCH** /api/v1/devices/{id} | 
[**get_device_handler**](DeviceHandlerApi.md#get_device_handler) | **GET** /api/v1/devices/{id} | 



## create_device_handler

> models::DeviceModel create_device_handler(create_device_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_device_schema** | [**CreateDeviceSchema**](CreateDeviceSchema.md) | Create device | [required] |

### Return type

[**models::DeviceModel**](DeviceModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_device_handler

> delete_device_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Device id for deleting device | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## device_list_handler

> models::PagedResponseDeviceModel device_list_handler(page, limit, order_by, desc, name)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | device name |  |

### Return type

[**models::PagedResponseDeviceModel**](PagedResponse_DeviceModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## device_list_search_handler

> models::PagedResponseDeviceModel device_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by device name |  |

### Return type

[**models::PagedResponseDeviceModel**](PagedResponse_DeviceModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_device_handler

> models::DeviceModel edit_device_handler(id, update_device_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Device id for patching person | [required] |
**update_device_schema** | [**UpdateDeviceSchema**](UpdateDeviceSchema.md) | Update device | [required] |

### Return type

[**models::DeviceModel**](DeviceModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_device_handler

> models::DeviceModel get_device_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Device id for fetching device | [required] |

### Return type

[**models::DeviceModel**](DeviceModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

